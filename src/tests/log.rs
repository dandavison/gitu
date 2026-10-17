use super::*;
use crate::item_data::ItemData;
use std::time::{Duration, Instant};

fn setup(ctx: TestContext) -> TestContext {
    commit(&ctx.dir, "third commit", "");
    commit(&ctx.dir, "second commit", "");
    commit(&ctx.dir, "first commit", "");
    with_renderer(ctx)
}

#[test]
fn limit_prompt() {
    snapshot!(setup(setup_clone!()), "l-n-n");
}

#[test]
fn limit_set_10() {
    snapshot!(setup(setup_clone!()), "l-n-n10<enter>");
}

#[test]
fn limit_invalid() {
    snapshot!(setup(setup_clone!()), "l-n-nfff<enter>");
}

#[test]
fn limit_2_commits() {
    snapshot!(setup(setup_clone!()), "l-n-n2<enter>l");
}

#[test]
fn limit_2_commits_other() {
    snapshot!(setup(setup_clone!()), "l-n-n2<enter>l");
}

#[test]
fn grep_prompt() {
    snapshot!(setup(setup_clone!()), "l-F");
}

#[test]
fn grep_set_example() {
    snapshot!(setup(setup_clone!()), "l-Fexample<enter>");
}

#[test]
fn grep_second() {
    snapshot!(setup(setup_clone!()), "l-Fsecond<enter>l");
}

#[test]
fn grep_no_match() {
    snapshot!(setup(setup_clone!()), "l-Fdoesntexist<enter>l");
}

#[test]
fn grep_second_other() {
    snapshot!(setup(setup_clone!()), "l-Fsecond<enter>omain<enter>");
}

#[test]
fn log_other_prompt() {
    snapshot!(setup(setup_clone!()), "lljlo");
}

#[test]
fn log_other() {
    snapshot!(setup(setup_clone!()), "lljlo<enter>");
}

#[test]
fn log_other_input() {
    snapshot!(setup(setup_clone!()), "lomain~1<enter>");
}

#[test]
fn log_other_invalid() {
    snapshot!(setup(setup_clone!()), "lo <enter>");
}

#[test]
fn log_empty_branch() {
    // Regression for #262: showing the log of a branch with no commits used to
    // panic ("index out of bounds") because the log screen had no items but the
    // cursor still indexed into it.
    let mut ctx = setup_clone!();
    run(&ctx.dir, &["rm", "-rf", ".git"]);
    run(&ctx.dir, &["rm", "initial-file"]);
    run(&ctx.dir, &["git", "init", "--initial-branch=main"]);

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

/// Give `git log` the user's arguments, as `general.log_args` does.
pub(super) fn with_log_args(mut ctx: TestContext, args: &[&str]) -> TestContext {
    ctx.config().general.log_args = args.iter().map(|arg| arg.to_string()).collect();
    ctx
}

/// A format printing each commit over several rows, as the renderer marks them.
/// Dates are absolute so the output doesn't drift with the wall clock.
pub(super) const MULTI_ROW_LOG_ARGS: &[&str] = &[
    "--date=short",
    "--format=%n\u{25b8} %h %an %ad%C(auto)%d%C(reset)%n    %s",
];

/// The command a view is the output of, as `:` offers it for editing.
fn git_command_of(app: &crate::app::App) -> Option<String> {
    app.screen()
        .git_command
        .as_ref()
        .map(|git| git.borrow().line(None))
}

/// The log opened from gitu is the view `git log` gives on the command line,
/// with the log menu's arguments.
#[test]
fn the_log_is_the_output_of_git_log() {
    let mut ctx = setup(setup_clone!());
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("l-Fsecond<enter>l"));

    assert_eq!(
        git_command_of(&app),
        Some("git log -n256 --extended-regexp --grep=second".into())
    );
}

#[test]
fn log_args_are_given_to_git_log() {
    let mut ctx = with_log_args(setup(setup_clone!()), &["--format=logged %s"]);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("ll"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("logged add second commit"), "{buffer}");
}

/// The log is drawn once the renderer has drawn its first screen, as the
/// command-line `git log` is.
#[test]
fn the_log_is_drawn_before_the_renderer_finishes() {
    let mut ctx = setup_clone!();
    for i in 1..=40 {
        commit(&ctx.dir, &format!("file-{i:02}"), "");
    }
    ctx.config().general.diff_renderer.command = super::pager::pausing_renderer();

    let started = Instant::now();
    let mut app = ctx.init_app_streaming_with_args(Default::default());
    ctx.update(&mut app, keys("ll"));

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(ctx.redact_buffer().contains("add file-40"));
}

#[test]
fn log_move_down_steps_a_whole_commit() {
    snapshot!(
        with_log_args(setup(setup_clone!()), MULTI_ROW_LOG_ARGS),
        "lljj"
    );
}

#[test]
fn log_move_next_section_steps_a_whole_commit() {
    snapshot!(
        with_log_args(setup(setup_clone!()), MULTI_ROW_LOG_ARGS),
        "ll<alt+j><alt+j>"
    );
}

#[test]
fn log_move_prev_section_steps_a_whole_commit() {
    snapshot!(
        with_log_args(setup(setup_clone!()), MULTI_ROW_LOG_ARGS),
        "lljj<alt+k>"
    );
}

#[test]
fn log_fold_hides_the_commit_rows() {
    snapshot!(
        with_log_args(setup(setup_clone!()), MULTI_ROW_LOG_ARGS),
        "ll<tab>"
    );
}

#[test]
fn log_show_acts_on_the_selected_commit() {
    snapshot!(setup(setup_clone!()), "llj<enter>");
}

#[test]
fn log_copy_hash_of_the_selected_commit() {
    snapshot!(setup(setup_clone!()), "lljy");
}

#[test]
fn log_with_stat() {
    // Extra rows a commit prints after its message stay part of that commit.
    snapshot!(
        with_log_args(
            setup(setup_clone!()),
            &["--stat", "--format=\u{25b8} %h %s"]
        ),
        "llj"
    );
}

/// A renderer that draws a rule above each commit, as delta does with
/// `commit-decoration-style = ol`.
#[test]
fn log_divider_is_not_the_cursor_line() {
    let mut ctx = with_log_args(setup(setup_clone!()), &["--format=\u{25b8} %h %s"]);
    ctx.config().general.diff_renderer.command = [
        "awk",
        r#"/^▸ /{printf "\033]1717;1;C;;;%s\033\\\n─────\n", $2} {print}"#,
    ]
    .map(String::from)
    .to_vec();
    snapshot!(ctx, "llj");
}

#[test]
fn log_keeps_the_first_commit_when_the_renderer_greets_on_its_line() {
    // A renderer emits its OSC-1717 handshake as its first output, which shares
    // a line with the first commit's record (delta does this). The newest commit
    // must still be a commit: selectable, cursor on it.
    let mut ctx = with_log_args(setup(setup_clone!()), &["--format=\u{25b8} %h %s"]);
    ctx.config().general.diff_renderer.command = [
        "awk",
        r#"BEGIN{printf "\033]1717;1\033\\"} /^▸ /{printf "\033]1717;1;C;;;%s\033\\", $2} {print}"#,
    ]
    .map(String::from)
    .to_vec();

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

/// Turning the page takes the cursor with it: the commit selected afterwards is
/// one of those now on screen, so the next op acts on what is being looked at.
#[test]
fn paging_lands_on_a_commit_of_the_new_page() {
    let mut ctx = with_renderer(setup_clone!());
    for i in 1..=40 {
        commit(&ctx.dir, &format!("file-{i:02}"), "");
    }

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    let top = selected_summary(&app);

    ctx.update(&mut app, keys("<pagedown>"));
    let paged = selected_summary(&app);
    assert_ne!(top, paged, "the cursor stayed on the first commit");
    assert!(
        ctx.redact_buffer().contains(&paged),
        "selected {paged} is off screen:\n{}",
        ctx.redact_buffer()
    );

    ctx.update(&mut app, keys("<pageup>"));
    assert_eq!(top, selected_summary(&app), "paging back missed the commit");
}

fn selected_summary(app: &crate::app::App) -> String {
    match &app.state.screens.last().unwrap().get_selected_item().data {
        ItemData::Commit { summary, .. } => summary.clone(),
        data => panic!("not a commit: {data:?}"),
    }
}

/// Three commits, the newest changing a file the oldest added, paged through
/// a renderer that marks commits and annotates diff lines, as delta does.
fn log_of_patches(ctx: &mut TestContext) -> crate::app::App {
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "b.rs", "beta\n");
    commit(&ctx.dir, "a.rs", "alpha changed\n");
    ctx.config().general.diff_renderer.command = super::pager::osc_1717_passthrough();
    ctx.init_app_as_pager_of(&["git", "log", "-p", "-3"])
}

/// Each item of the view, as the summary of the commit it is under and,
/// within a diff, the file and line it is.
fn under_commits(app: &crate::app::App) -> Vec<(String, String)> {
    let mut summary = String::new();
    app.screen()
        .items()
        .iter()
        .filter_map(|item| match &item.data {
            ItemData::Commit { summary: of, .. } => {
                summary = of.clone();
                None
            }
            ItemData::HunkLine {
                diff,
                file_i,
                hunk_i,
                line_range,
                ..
            } => Some((
                summary.clone(),
                format!(
                    "{} {}",
                    diff.file_diffs[*file_i].header.new_file.fmt(&diff.text),
                    diff.hunk_content(*file_i, *hunk_i)[line_range.clone()].trim_end()
                ),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_log_of_patches_has_each_diff_under_its_commit() {
    let mut ctx = setup_clone!();
    let app = log_of_patches(&mut ctx);

    let lines = under_commits(&app);
    let lines: Vec<(&str, &str)> = lines
        .iter()
        .map(|(commit, line)| (commit.as_str(), line.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            ("modify a.rs", "a.rs -alpha"),
            ("modify a.rs", "a.rs +alpha changed"),
            ("add b.rs", "b.rs +beta"),
            ("add a.rs", "a.rs +alpha"),
        ]
    );
}

#[test]
fn a_log_of_patches_nests_each_diff_under_its_commit() {
    let mut ctx = setup_clone!();
    let app = log_of_patches(&mut ctx);

    let items = app.screen().items();
    let commit = items
        .iter()
        .position(
            |item| matches!(&item.data, ItemData::Commit { summary, .. } if summary == "add b.rs"),
        )
        .unwrap();
    let file = commit
        + items[commit..]
            .iter()
            .position(|item| matches!(&item.data, ItemData::Delta { .. }))
            .unwrap();
    assert!(items[file].depth > items[commit].depth);
}

#[test]
fn folding_a_commit_in_a_log_of_patches_hides_its_diff() {
    let mut ctx = setup_clone!();
    let mut app = log_of_patches(&mut ctx);

    ctx.update(&mut app, keys("gg<tab>"));

    let buffer = ctx.redact_buffer();
    assert!(!buffer.contains("alpha changed"), "{buffer}");
    assert!(buffer.contains("+beta"), "{buffer}");
}

/// A hunk of a commit in the log is the commit's: reversing it undoes that
/// commit's change in the working tree.
#[test]
fn a_hunk_in_a_log_of_patches_reverses_its_commits_change() {
    let mut ctx = setup_clone!();
    let mut app = log_of_patches(&mut ctx);

    ctx.update(&mut app, keys("/alpha changed<enter>kv"));

    assert_eq!(fs::read_to_string(ctx.dir.join("a.rs")).unwrap(), "alpha\n");
}

/// A renderer that marks commits but says nothing of the diff's lines gives
/// gitu no way to place them, so the log is its commits and their rows.
#[test]
fn a_log_of_patches_drawn_without_line_records_is_its_commits() {
    let mut ctx = with_renderer(setup_clone!());
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "b.rs", "beta\n");
    let app = ctx.init_app_as_pager_of(&["git", "log", "-p", "-2"]);

    let commits = app
        .screen()
        .items()
        .iter()
        .filter(|item| matches!(item.data, ItemData::Commit { .. }))
        .count();
    assert_eq!(commits, 2);
    assert!(ctx.redact_buffer().contains("+beta"));
}
