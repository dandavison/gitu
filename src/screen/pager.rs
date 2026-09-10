//! The screen `gitu --pager` shows: a patch that arrived on stdin rather than
//! one gitu asked git for.
//!
//! Set as git's pager, gitu is handed the very bytes `git diff` / `git show`
//! produce, so the patch text is the source of truth exactly as it is
//! everywhere else — the difference is only where it came from. git colours
//! what it writes to a pager, so the text is stripped of ANSI before it is
//! parsed; the colours in the view are the renderer's, applied afterwards.

use super::Screen;
use crate::{
    Res,
    config::Config,
    git::{
        self,
        diff::{Diff, DiffType},
    },
    gitu_diff,
    items::{self, Item, RenderParams},
};
use git2::Repository;
use std::{rc::Rc, sync::Arc};

pub(crate) fn create(
    config: Arc<Config>,
    repo: Rc<Repository>,
    params: RenderParams,
    patch: String,
) -> Res<Screen> {
    let source = Source::of(&repo, patch)?;

    Screen::new(
        Arc::clone(&config),
        params,
        Box::new(move |params: RenderParams| source.items(&config, &repo, &params)),
    )
}

/// Where the rows come from each time the screen is rebuilt.
enum Source {
    /// A patch recognised as one git can be asked for again: staging a hunk
    /// from it changes what it shows, as it does on the status screen.
    Live(fn(&Repository) -> Res<Diff>),
    /// Anything else — a commit, two revs, another repo's patch, output that is
    /// no patch at all. There is nothing to ask again, so it stays as it came.
    Fixed { patch: String, diff: Rc<Diff> },
}

impl Source {
    /// git tells its pager nothing about the command that produced the patch,
    /// so the two diffs git can be asked for again are asked for and compared
    /// against it. Recognising the patch is what makes it a live view, and it
    /// is also what decides which ops it admits (see [`DiffType`]).
    fn of(repo: &Repository, patch: String) -> Res<Self> {
        let text = crate::diff_colorizer::strip_ansi(&patch);

        for ask_git in [
            git::diff_unstaged as fn(&Repository) -> Res<Diff>,
            git::diff_staged,
        ] {
            if ask_git(repo)?.text == text {
                return Ok(Source::Live(ask_git));
            }
        }

        Ok(Source::Fixed {
            diff: Rc::new(Diff {
                file_diffs: gitu_diff::Parser::new(&text)
                    .parse_diff()
                    .unwrap_or_default(),
                text,
                diff_type: DiffType::TreeToTree,
                commit: None,
            }),
            patch,
        })
    }

    fn items(&self, config: &Config, repo: &Repository, params: &RenderParams) -> Res<Vec<Item>> {
        match self {
            Source::Live(ask_git) => Ok(diff_items(config, params, &Rc::new(ask_git(repo)?))),
            // Nothing parsed as a patch, so there is nothing to fold or stage:
            // show what arrived, as it arrived.
            Source::Fixed { patch, diff } if diff.file_diffs.is_empty() => {
                Ok(items::plain_rows(patch))
            }
            // `git show` and `git log -p` open with the commit their diff is of.
            // It belongs to no file, so it is shown as it arrived, above the
            // diff that gitu does structure.
            Source::Fixed { patch, diff } => {
                let mut items = items::plain_rows_before_first_file_diff(patch);
                items.extend(diff_items(config, params, diff));
                Ok(items)
            }
        }
    }
}

fn diff_items(config: &Config, params: &RenderParams, diff: &Rc<Diff>) -> Vec<Item> {
    items::create_diff_items(config, params, diff, 0, false, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::item_data::ItemData;
    use crate::repo_setup_clone;
    use crate::tests::helpers::RepoTestContext;
    use stdext::function_name;

    const PATCH: &str = "diff --git a/f.rs b/f.rs\n\
                         index 1..2 100644\n\
                         --- a/f.rs\n\
                         +++ b/f.rs\n\
                         @@ -1,3 +1,3 @@\n\
                         \x20keep\n\
                         -gone\n\
                         +added\n";

    fn items_of(repo: &Repository, patch: &str) -> Vec<ItemData> {
        let config = config::init_test_config().unwrap();
        Source::of(repo, patch.to_string())
            .unwrap()
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
                size: ratatui::layout::Size::new(80, 20),
                features: Rc::from([]),
            },
            patch.to_string(),
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
}
