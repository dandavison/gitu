//! The screen `gitu --pager` shows: a patch that arrived on stdin rather than
//! one gitu asked git for.
//!
//! Set as git's pager, gitu is handed the very bytes `git diff` / `git show`
//! produce, so the patch text is the source of truth exactly as it is
//! everywhere else — the difference is only where it came from. git colours
//! what it writes to a pager, so the text is stripped of ANSI before it is
//! parsed; the colours in the view are the renderer's, applied afterwards.

use super::Screen;
use crate::style::{Modifier, Style};
use crate::{
    Res,
    calling_process::GitCommand,
    capped_input::Capped,
    config::Config,
    error::Error,
    git::diff::{Diff, DiffType},
    gitu_diff,
    items::{self, Item, RenderParams},
};
use git2::Repository;
use std::{cell::RefCell, io::Read, process::Stdio, rc::Rc, sync::Arc, thread};

/// What git handed gitu as its pager: its output, and the argv of the command
/// that produced it where gitu could find it.
pub struct Paged {
    pub text: String,
    pub git_argv: Option<Vec<String>>,
    /// Whether reading the input stopped at the byte limit rather than at its
    /// end (see [`Capped`]).
    pub truncated: bool,
}

pub(crate) fn create(
    config: Arc<Config>,
    repo: Rc<Repository>,
    params: RenderParams,
    paged: Paged,
) -> Res<Screen> {
    let source = Source::of(paged, &config.general.hide);
    let git_command = source.git_command();

    let mut screen = Screen::new(
        Arc::clone(&config),
        params,
        Box::new(move |params: RenderParams| source.items(&config, &repo, &params)),
    )?;
    screen.git_command = git_command;

    Ok(screen)
}

/// Where the rows come from each time the screen is rebuilt.
enum Source {
    /// A question git can be put again, gitu having found the command that
    /// produced the patch. Asking again is what makes it a live view — staging
    /// a hunk changes what it shows — and the question itself can be edited,
    /// so what comes back is classified afresh on every ask.
    Live {
        git: Rc<RefCell<GitCommand>>,
        /// The command as git put it, to say when what is being asked is no
        /// longer that.
        found: String,
    },
    /// A patch gitu found no command for: a saved patch file, another repo's,
    /// one piped in from a process already gone. It stays as it came, and gitu
    /// structures it.
    Patch { diff: Rc<Diff>, truncated: bool },
    /// Output with no diff in it: a log, a blame, a grep, a man page. gitu has
    /// no structure of its own to impose, so the renderer draws it and what it
    /// says about the rows it drew decides what they are.
    Unstructured { text: String, truncated: bool },
}

impl Source {
    /// The command gitu found is what the patch is: it says what the patch is a
    /// diff of, and so which ops it admits (see [`DiffType`]), and it can be
    /// put again. Without one, the patch stays as it came and gitu structures
    /// whatever of it is a diff.
    ///
    /// The files the user has said to `hide` are dropped from the question
    /// before it is first asked, so the command holds them like any other
    /// pathspec: `:` shows them, and `_` takes them back.
    fn of(paged: Paged, hide: &[String]) -> Self {
        if let Some(git) = paged.git_argv.and_then(GitCommand::of) {
            let mut patterns = git.file_patterns();
            patterns.extend(hide.iter().map(|glob| format!("!{glob}")));

            return Source::Live {
                found: git.line(None),
                git: Rc::new(RefCell::new(git.asking_for(&patterns))),
            };
        }

        let text = crate::diff_renderer::strip_ansi(&paged.text);
        let file_diffs = gitu_diff::Parser::new(&text)
            .parse_diff()
            .unwrap_or_default();

        if file_diffs.is_empty() {
            return Source::Unstructured {
                text: paged.text,
                truncated: paged.truncated,
            };
        }

        Source::Patch {
            diff: Rc::new(Diff {
                file_diffs,
                text,
                diff_type: DiffType::TreeToTree,
                commit: None,
            }),
            truncated: paged.truncated,
        }
    }

    /// The command being asked, where there is one to edit.
    fn git_command(&self) -> Option<Rc<RefCell<GitCommand>>> {
        match self {
            Source::Live { git, .. } => Some(Rc::clone(git)),
            Source::Patch { .. } | Source::Unstructured { .. } => None,
        }
    }

    fn items(&self, config: &Config, repo: &Repository, params: &RenderParams) -> Res<Vec<Item>> {
        let limit = config.general.max_input_bytes;
        let (diff, mut items, truncated) = match self {
            Source::Live { git, found } => {
                let line = git.borrow().line(params.context.as_deref());
                let asked = ask_git(&git.borrow(), repo, params.context.as_deref(), limit)?;
                (
                    Rc::new(asked.diff),
                    asked_rows(&line, found),
                    asked.truncated,
                )
            }
            Source::Patch { diff, truncated } => (Rc::clone(diff), Vec::new(), *truncated),
            Source::Unstructured { text, truncated } => {
                let mut items = rows(config, repo, params, text);
                items.extend(truncated_rows(*truncated, limit));
                return Ok(items);
            }
        };

        // What came back has no diff in it: a log, a blame, a grep, a man page.
        // gitu has no structure of its own to impose, so the renderer draws it.
        if diff.file_diffs.is_empty() {
            items.extend(rows(config, repo, params, &diff.text));
        } else {
            items.extend(preamble_items(config, params, &diff.text));
            items.extend(diff_items(config, params, &diff));
        }

        items.extend(truncated_rows(truncated, limit));
        Ok(items)
    }
}

/// What putting the command again said, and whether it was still saying it when
/// reading stopped.
struct Asked {
    diff: Diff,
    truncated: bool,
}

/// Put the command again, for however much of the file around each change is
/// being asked for now, reading it to the same limit as the piped input.
fn ask_git(git: &GitCommand, repo: &Repository, context: Option<&str>, limit: u64) -> Res<Asked> {
    let mut child = git
        .command(context)
        .current_dir(repo.workdir().ok_or(Error::NoRepoWorkdir)?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::GitDiff)?;

    // Read stderr on its own thread: a command filling that pipe while gitu is
    // reading stdout would otherwise deadlock.
    let mut pipe = child.stderr.take().expect("stderr was piped");
    let complaint = thread::spawn(move || {
        let mut said = String::new();
        let _ = pipe.read_to_string(&mut said);
        said
    });

    let output = Capped::read(child.stdout.take().expect("stdout was piped"), limit)
        .map_err(Error::GitDiff)?;
    if output.truncated {
        // The rest of it is not going to be read, so it is not left to be
        // written either.
        let _ = child.kill();
    }

    let status = child.wait().map_err(Error::GitDiff)?;
    // A command the user can edit is a command the user can get wrong, and an
    // empty screen does not say what git thought of it. (`git diff --exit-code`
    // answers with a status too, and a killed command has one as well, so a
    // command that answered at all is taken to have worked.)
    if !status.success() && output.text.is_empty() {
        let complaint = complaint.join().unwrap_or_default();
        return Err(Error::GitRefusedTheCommand(
            complaint.lines().next().unwrap_or_default().to_owned(),
        ));
    }

    let text = output.text;
    Ok(Asked {
        diff: Diff {
            file_diffs: gitu_diff::Parser::new(&text)
                .parse_diff()
                .unwrap_or_default(),
            diff_type: git.diff_type(),
            commit: commit_named_by(&text),
            text,
        },
        truncated: output.truncated,
    })
}

/// One line saying what is being asked, and only when that is not what git was
/// asked: a view with files hidden from it must not pass for the whole patch.
/// When gitu is asking git's own question there is nothing to say.
fn asked_rows(line: &str, found: &str) -> Vec<Item> {
    if line == found {
        return Vec::new();
    }

    vec![dim_row(line.to_owned())]
}

/// One line saying that what is shown is not all of the output, for output that
/// stopped at the byte limit rather than at its end. It goes last, where the
/// reading stops: nobody who does not get that far needs to be told. What is
/// past the limit was not read, so there is nothing to say about how much of it
/// there was.
fn truncated_rows(truncated: bool, limit: u64) -> Vec<Item> {
    if !truncated {
        return Vec::new();
    }

    vec![dim_row(format!(
        "input truncated at {limit} bytes (general.max_input_bytes)"
    ))]
}

/// A row of gitu's own, saying something about the view rather than being part
/// of it.
fn dim_row(text: String) -> Item {
    Item {
        unselectable: true,
        rendered: Some(Rc::new(vec![(
            text,
            Style {
                add_modifier: Modifier::DIM,
                ..Style::new()
            },
        )])),
        ..Default::default()
    }
}

/// Text the renderer draws and gitu does not structure. A renderer that says
/// which commit a row belongs to has turned it into a log; one that says
/// nothing has just drawn it.
fn rows(config: &Config, repo: &Repository, params: &RenderParams, text: &str) -> Vec<Item> {
    let rendered = render(config, params, text);
    items::rendered_log_items(repo, &rendered).unwrap_or_else(|| items::plain_rows(&rendered))
}

/// The commit a patch from `git show` is of, named in its first line.
fn commit_named_by(text: &str) -> Option<String> {
    let id = text
        .lines()
        .next()?
        .strip_prefix("commit ")?
        .split_whitespace()
        .next()?;

    id.chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| id.to_owned())
}

/// The commit a patch opens with, as the renderer draws it. It belongs to no
/// file, so gitu has no structure to give it. A patch whose first line already
/// begins a file's diff opens with nothing.
fn preamble_items(config: &Config, params: &RenderParams, text: &str) -> Vec<Item> {
    let end = match text.find("\ndiff --git ") {
        Some(i) if !text.starts_with("diff --git ") => i + 1,
        _ => return Vec::new(),
    };

    items::plain_rows(&render(config, params, &text[..end]))
}

fn diff_items(config: &Config, params: &RenderParams, diff: &Rc<Diff>) -> Vec<Item> {
    items::create_diff_items(config, params, diff, 0, false, None)
}

/// `text` as the renderer draws it, or as it arrived when there is no renderer
/// to draw it — which is also how output that already came through one keeps
/// the records it came with.
fn render(config: &Config, params: &RenderParams, text: &str) -> String {
    if !config.general.diff_renderer.enabled {
        return text.to_owned();
    }

    crate::diff_renderer::run(
        &config.general.diff_renderer.command,
        Some(text),
        params,
        None,
    )
    .unwrap_or_else(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::item_data::ItemData;
    use crate::repo_setup_clone;
    use crate::tests::helpers::{RepoTestContext, run};
    use stdext::function_name;

    const PATCH: &str = "diff --git a/f.rs b/f.rs\n\
                         index 1..2 100644\n\
                         --- a/f.rs\n\
                         +++ b/f.rs\n\
                         @@ -1,3 +1,3 @@\n\
                         \x20keep\n\
                         -gone\n\
                         +added\n";

    /// Input as it arrives when the limit did not stop it.
    fn paged(text: &str, git_argv: Option<Vec<String>>) -> Paged {
        Paged {
            text: text.to_owned(),
            git_argv,
            truncated: false,
        }
    }

    fn items_of(repo: &Repository, patch: &str) -> Vec<ItemData> {
        let config = config::init_test_config().unwrap();
        Source::of(paged(patch, None), &[])
            .items(&config, repo, &Default::default())
            .unwrap()
            .into_iter()
            .map(|item| item.data)
            .collect()
    }

    fn screen_of(ctx: &RepoTestContext, patch: &str) -> Screen {
        create(
            Arc::new(config::init_test_config().unwrap()),
            Rc::new(Repository::open(&ctx.dir).unwrap()),
            RenderParams {
                size: (80, 20),
                features: Rc::from([]),
                context: None,
            },
            paged(patch, None),
        )
        .unwrap()
    }

    #[test]
    fn a_piped_patch_becomes_a_file_a_hunk_and_its_lines() {
        let ctx = repo_setup_clone!();
        let items = items_of(&ctx.local_repo, PATCH);

        assert!(
            matches!(items.first(), Some(ItemData::Delta { .. })),
            "got {items:?}"
        );
        assert!(items.iter().any(|d| matches!(d, ItemData::Hunk { .. })));
        assert_eq!(
            items
                .iter()
                .filter(|d| matches!(d, ItemData::HunkLine { .. }))
                .count(),
            3
        );
    }

    /// git colours what it writes to its pager, and the parser reads none of it
    /// through the escapes, so they have to come off first.
    #[test]
    fn a_coloured_patch_parses_the_same_as_a_plain_one() {
        let ctx = repo_setup_clone!();
        let coloured = "\x1b[1mdiff --git a/f.rs b/f.rs\x1b[m\n\
             \x1b[1mindex 1..2 100644\x1b[m\n\
             \x1b[1m--- a/f.rs\x1b[m\n\
             \x1b[1m+++ b/f.rs\x1b[m\n\
             \x1b[36m@@ -1,3 +1,3 @@\x1b[m\n\
             \x20keep\n\
             \x1b[31m-gone\x1b[m\n\
             \x1b[32m+added\x1b[m\n";

        assert_eq!(
            items_of(&ctx.local_repo, coloured).len(),
            items_of(&ctx.local_repo, PATCH).len()
        );
    }

    /// A patch from `git show` opens with the commit it is the diff of. That is
    /// not part of any file's diff, and dropping it loses who wrote the change
    /// and why.
    #[test]
    fn a_commit_patch_keeps_its_message_above_the_diff() {
        let ctx = repo_setup_clone!();
        let patch = format!(
            "commit 0123456789abcdef\n\
                             Author: Author Name <author@email.com>\n\
                             \n\
                             \x20   add initial-file\n\
                             \n\
                             {PATCH}"
        );

        let screen = screen_of(&ctx, &patch);

        let rows = screen.row_texts();
        assert!(
            rows.starts_with(&[
                "commit 0123456789abcdef".to_string(),
                "Author: Author Name <author@email.com>".to_string(),
                String::new(),
                "    add initial-file".to_string(),
                String::new(),
            ]),
            "got {rows:?}"
        );
        assert!(
            items_of(&ctx.local_repo, &patch)
                .iter()
                .any(|data| matches!(data, ItemData::HunkLine { .. })),
            "the diff below it is still a diff"
        );
    }

    /// A patch that opens with a file's diff has nothing above it, and every
    /// file it touches is shown once.
    #[test]
    fn a_patch_of_several_files_shows_each_of_them_once() {
        let ctx = repo_setup_clone!();
        let patch = format!("{PATCH}{}", PATCH.replace("f.rs", "g.rs"));

        let items = items_of(&ctx.local_repo, &patch);

        assert!(
            matches!(items.first(), Some(ItemData::Delta { .. })),
            "got {items:?}"
        );
        assert_eq!(
            items
                .iter()
                .filter(|d| matches!(d, ItemData::Delta { .. }))
                .count(),
            2
        );
        assert_eq!(
            items
                .iter()
                .filter(|d| matches!(d, ItemData::HunkLine { .. }))
                .count(),
            6
        );
    }

    /// Unstructured rows are rows like any other: each one takes the cursor.
    #[test]
    fn the_cursor_moves_between_unstructured_rows() {
        let ctx = repo_setup_clone!();
        let mut screen = screen_of(&ctx, "one\ntwo\nthree\n");
        assert_eq!(screen.row_texts().len(), 3, "{:?}", screen.row_texts());

        screen.select_next(crate::screen::NavMode::IncludeSubLines);

        assert_eq!(selected_row(&screen), "two");
    }

    fn selected_row(screen: &Screen) -> String {
        screen
            .get_selected_item()
            .rendered
            .as_ref()
            .map(|row| row.iter().map(|(text, _)| text.as_str()).collect())
            .unwrap_or_default()
    }

    /// Input that is not a git patch offers no structure rather than failing.
    #[test]
    fn output_that_is_not_a_patch_yields_no_files() {
        let ctx = repo_setup_clone!();
        assert!(
            !items_of(&ctx.local_repo, "this is not a diff\nnor is this\n")
                .iter()
                .any(|data| matches!(
                    data,
                    ItemData::Delta { .. } | ItemData::Hunk { .. } | ItemData::HunkLine { .. }
                ))
        );
        assert!(items_of(&ctx.local_repo, "").is_empty());
    }

    /// ... and is shown as it arrived, rather than as an empty screen.
    #[test]
    fn output_that_is_not_a_patch_is_still_readable() {
        let ctx = repo_setup_clone!();
        let grep = "m.rs:2:    let alpha = 1;\nm.rs:3:    let gamma = 42;\n";
        let screen = screen_of(&ctx, grep);

        assert_eq!(
            screen.row_texts(),
            ["m.rs:2:    let alpha = 1;", "m.rs:3:    let gamma = 42;"]
        );
    }

    /// A diff against another rev is as live as any other: git names the
    /// command it ran, so gitu can run it again and ask for more of the file
    /// around each change.
    #[test]
    fn the_context_of_a_diff_against_a_rev_can_be_widened() {
        let ctx = repo_setup_clone!();
        let lines = (1..=21).map(|n| format!("line {n}\n")).collect::<String>();
        write(&ctx, &lines);
        run(&ctx.dir, &["git", "add", "f.txt"]);
        run(&ctx.dir, &["git", "commit", "-m", "add f.txt"]);
        run(&ctx.dir, &["git", "checkout", "-b", "topic"]);
        write(&ctx, &lines.replace("line 11\n", "LINE 11\n"));
        run(&ctx.dir, &["git", "commit", "-am", "change f.txt"]);

        let patch = run(&ctx.dir, &["git", "diff", "main"]);
        let argv = ["git", "diff", "main"].map(String::from).to_vec();

        assert!(
            hunk_lines(&ctx, &patch, &argv, Some("-U5")) > hunk_lines(&ctx, &patch, &argv, None)
        );
    }

    /// Output with no diff in it is still a question gitu holds, so editing it
    /// into one that has a diff turns rows into files and hunks.
    #[test]
    fn an_edit_that_asks_for_a_diff_gets_one() {
        let ctx = repo_setup_clone!();
        let config = config::init_test_config().unwrap();
        let repo = Repository::open(&ctx.dir).unwrap();
        let source = Source::of(
            paged(
                &run(&ctx.dir, &["git", "log"]),
                Some(["git", "log"].map(String::from).to_vec()),
            ),
            &[],
        );
        let items = |source: &Source| {
            source
                .items(&config, &repo, &Default::default())
                .unwrap()
                .into_iter()
                .map(|item| item.data)
                .collect::<Vec<_>>()
        };

        assert!(
            !items(&source)
                .iter()
                .any(|data| matches!(data, ItemData::HunkLine { .. })),
            "a log has no diff in it"
        );

        let git = source.git_command().expect("the command was found");
        let edited = git.borrow().edited("git log -p").unwrap();
        *git.borrow_mut() = edited;

        assert!(
            items(&source)
                .iter()
                .any(|data| matches!(data, ItemData::HunkLine { .. })),
            "a log with patches does"
        );
    }

    /// Generated files and fixtures are noise in every patch, so saying so once
    /// in config drops them before the question is first asked.
    #[test]
    fn the_files_config_hides_are_gone_from_the_first_ask() {
        let ctx = repo_setup_clone!();
        let config = config::init_test_config().unwrap();
        let repo = Repository::open(&ctx.dir).unwrap();
        for file in ["a.rs", "a.pb.rs"] {
            std::fs::write(ctx.dir.join(file), "generated\n").unwrap();
        }
        run(&ctx.dir, &["git", "add", "."]);

        let source = Source::of(
            paged(
                &run(&ctx.dir, &["git", "diff", "--cached"]),
                Some(["git", "diff", "--cached"].map(String::from).to_vec()),
            ),
            &["*.pb.rs".to_owned()],
        );
        let files = source
            .items(&config, &repo, &Default::default())
            .unwrap()
            .iter()
            .filter_map(|item| match &item.data {
                ItemData::Delta { diff, file_i, .. } => Some(
                    diff.file_diffs[*file_i]
                        .header
                        .new_file
                        .fmt(&diff.text)
                        .into_owned(),
                ),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(files, ["a.rs"]);
    }

    fn write(ctx: &RepoTestContext, content: &str) {
        std::fs::write(ctx.dir.join("f.txt"), content).unwrap();
    }

    /// How many lines of the diff the screen shows, for the patch git piped and
    /// the command it ran to make it.
    fn hunk_lines(
        ctx: &RepoTestContext,
        patch: &str,
        argv: &[String],
        context: Option<&str>,
    ) -> usize {
        let config = config::init_test_config().unwrap();
        let repo = Repository::open(&ctx.dir).unwrap();
        Source::of(paged(patch, Some(argv.to_vec())), &[])
            .items(
                &config,
                &repo,
                &RenderParams {
                    context: context.map(Rc::from),
                    ..Default::default()
                },
            )
            .unwrap()
            .iter()
            .filter(|item| matches!(item.data, ItemData::HunkLine { .. }))
            .count()
    }
}
