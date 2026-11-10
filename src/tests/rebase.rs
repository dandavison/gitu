use super::*;
use crate::cli::{Args, Commands};
use crate::tests::log::{MULTI_ROW_LOG_ARGS, with_log_args};
use std::path::{Path, PathBuf};

fn setup(ctx: TestContext) -> TestContext {
    run(&ctx.dir, &["git", "checkout", "-b", "other-branch"]);
    run(&ctx.dir, &["git", "checkout", "main"]);
    commit(&ctx.dir, "new-file", "hello");
    run(&ctx.dir, &["git", "checkout", "other-branch"]);
    ctx
}

#[test]
fn rebase_menu() {
    snapshot!(setup(setup_clone!()), "r");
}

#[test]
fn rebase_elsewhere_prompt() {
    snapshot!(setup(setup_clone!()), "re");
}

#[test]
fn rebase_elsewhere() {
    snapshot!(setup(setup_clone!()), "remain<enter>");
}

fn setup_todo(ctx: TestContext) -> TestContext {
    commit(&ctx.dir, "first-file", "");
    commit(&ctx.dir, "second-file", "");
    commit(&ctx.dir, "third-file", "");
    // One row per commit, so a commit's row is its summary.
    with_log_args(ctx, &["--format=▸ %h %s"])
}

/// Open the interactive rebase todo for the three commits: `l l` to the log,
/// down to the oldest of them, then the rebase menu's "interactively".
const OPEN_TODO: &str = "lljjri";

#[test]
fn rebase_todo() {
    snapshot!(setup_todo(setup_clone!()), OPEN_TODO);
}

#[test]
fn rebase_todo_move_down() {
    snapshot!(setup_todo(setup_clone!()), &format!("{OPEN_TODO}<alt+j>"));
}

#[test]
fn rebase_todo_move_up() {
    snapshot!(setup_todo(setup_clone!()), &format!("{OPEN_TODO}j<alt+k>"));
}

#[test]
fn rebase_todo_move_down_stops_at_the_end() {
    snapshot!(
        setup_todo(setup_clone!()),
        &format!("{OPEN_TODO}jj<alt+j><alt+j>")
    );
}

#[test]
fn rebase_todo_mark_squash() {
    snapshot!(setup_todo(setup_clone!()), &format!("{OPEN_TODO}js"));
}

#[test]
fn rebase_todo_mark_moves_on_to_the_next_entry() {
    let mut ctx = setup_todo(setup_clone!());
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(OPEN_TODO));
    assert!(cursor_row(&ctx).contains("add third-file"));

    // Marking works down the list, so the cursor follows onto the next entry.
    ctx.update(&mut app, keys("s"));
    assert!(
        cursor_row(&ctx).contains("add second-file"),
        "{}",
        ctx.redact_buffer()
    );
}

#[test]
fn rebase_todo_mark_stays_on_the_last_entry() {
    let mut ctx = setup_todo(setup_clone!());
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(&format!("{OPEN_TODO}jjs")));

    assert!(
        cursor_row(&ctx).starts_with("▌squash"),
        "{}",
        ctx.redact_buffer()
    );
    assert!(
        cursor_row(&ctx).contains("add first-file"),
        "{}",
        ctx.redact_buffer()
    );
}

/// Marking rebuilds the list without moving it on screen. The commits are one
/// row each, so the one after the next is already in view and nothing need
/// scroll to show it.
#[test]
fn rebase_todo_mark_leaves_the_view_where_it_is() {
    let mut ctx = with_log_args(setup_clone!(), &["--format=▸ %h %s"]);
    for i in 0..30 {
        commit(&ctx.dir, &format!("file-{i}"), "");
    }
    let todo = write_todo(&ctx, "HEAD~30..HEAD");

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("jjjjjjjjjjjjjjj"));
    let top = top_row(&ctx);

    ctx.update(&mut app, keys("s"));
    assert_eq!(top_row(&ctx), top, "the list stayed put");
}

/// The row the cursor sits on, as drawn.
fn cursor_row(ctx: &TestContext) -> String {
    ctx.redact_buffer()
        .lines()
        .find(|row| row.starts_with('▌'))
        .expect("a cursor row")
        .to_string()
}

/// Commits drawn on more rows than the screen keeps below the cursor, each
/// ending in a row that says it is the last.
const TALL_LOG_ARGS: &[&str] = &["--format=%n%n\u{25b8} %h%n%n    %s%n%n    last row of %s"];

/// A commit dragged down stops short of the bottom of the screen while there
/// are commits below it, so that the one it would pass next is in view whole.
#[test]
fn rebase_todo_move_down_keeps_the_next_entry_in_view() {
    let mut ctx = with_log_args(setup_clone!(), TALL_LOG_ARGS);
    for i in 0..30 {
        commit(&ctx.dir, &format!("file-{i}"), "");
    }
    let todo = write_todo(&ctx, "HEAD~30..HEAD");
    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));

    for _ in 0..20 {
        ctx.update(&mut app, keys("<alt+j>"));
        let buffer = ctx.redact_buffer();
        let below: Vec<&str> = buffer
            .lines()
            .skip_while(|row| !row.starts_with('▌'))
            .skip_while(|row| row.starts_with('▌'))
            .collect();
        assert!(
            below.iter().any(|row| row.contains("▸"))
                && below.iter().any(|row| row.contains("last row of")),
            "{buffer}"
        );
    }
}

/// Marking moves on to the next commit, keeping the one after it in view
/// whole, so that it shows whether the last one to mark has been reached.
#[test]
fn rebase_todo_mark_keeps_the_next_entry_in_view() {
    for mark in ["s", "f"] {
        let mut ctx = with_log_args(setup_clone!(), TALL_LOG_ARGS);
        for i in 0..30 {
            commit(&ctx.dir, &format!("file-{i}"), "");
        }
        let todo = write_todo(&ctx, "HEAD~30..HEAD");
        let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));

        for _ in 0..20 {
            ctx.update(&mut app, keys(mark));
            let buffer = ctx.redact_buffer();
            let below: Vec<&str> = buffer
                .lines()
                .skip_while(|row| !row.starts_with('▌'))
                .skip_while(|row| row.starts_with('▌'))
                .collect();
            assert!(
                below.iter().any(|row| row.contains("▸"))
                    && below.iter().any(|row| row.contains("last row of")),
                "{buffer}"
            );
        }
    }
}

/// ... and dragged up, it stops short of the top while there are commits above.
#[test]
fn rebase_todo_move_up_keeps_the_previous_entry_in_view() {
    let mut ctx = with_log_args(setup_clone!(), TALL_LOG_ARGS);
    for i in 0..30 {
        commit(&ctx.dir, &format!("file-{i}"), "");
    }
    let todo = write_todo(&ctx, "HEAD~30..HEAD");
    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("G"));

    for _ in 0..20 {
        ctx.update(&mut app, keys("<alt+k>"));
        let buffer = ctx.redact_buffer();
        let above: Vec<&str> = buffer
            .lines()
            .take_while(|row| !row.starts_with('▌'))
            .collect();
        assert!(
            above.iter().any(|row| row.contains("▸"))
                && above.iter().any(|row| row.contains("last row of")),
            "{buffer}"
        );
    }
}

fn top_row(ctx: &TestContext) -> String {
    ctx.redact_buffer().lines().next().unwrap().to_string()
}

#[test]
fn rebase_todo_quit_runs_nothing() {
    snapshot!(setup_todo(setup_clone!()), &format!("{OPEN_TODO}jdq"));
}

#[test]
fn rebase_todo_drop_and_start() {
    snapshot!(
        setup_todo(setup_clone!()),
        &format!("{OPEN_TODO}jd<enter>ll")
    );
}

#[test]
fn rebase_todo_reorder_and_start() {
    snapshot!(
        setup_todo(setup_clone!()),
        &format!("{OPEN_TODO}<alt+j><enter>ll")
    );
}

#[test]
fn rebase_todo_shows_the_rendered_log_rows() {
    // The commits look as they do in the log view, with the instruction in front.
    snapshot!(
        with_log_args(setup_todo(setup_clone!()), MULTI_ROW_LOG_ARGS),
        &format!("{OPEN_TODO}js")
    );
}

/// A renderer names a commit as the format printed it, usually abbreviated; the
/// todo's commits are still drawn with their rows.
#[test]
fn rebase_todo_shows_rows_a_renderer_marked_by_abbreviated_commit() {
    let mut ctx = with_log_args(setup_todo(setup_clone!()), &["--format=▸ %h %s"]);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys(OPEN_TODO));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("▸"), "{buffer}");
}

#[test]
fn rebase_todo_redraws_moved_renderer_hyperlinks() {
    let mut ctx = with_log_args(
        setup_todo(setup_clone!()),
        &["--format=▸ %h \x1b]8;;https://example.com/%H\x1b\\%s\x1b]8;;\x1b\\"],
    );

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(OPEN_TODO));

    let screen = ctx.physical_screen();
    for rev in ["HEAD", "HEAD~1", "HEAD~2"] {
        let commit = run(&ctx.dir, &["git", "log", "-1", "--format=%h %s", rev]);
        assert!(screen.contains(&format!("▸ {}", commit.trim())), "{screen}");
    }
}

/// The list git writes for `rebase -i`, oldest first, as its sequence editor
/// receives it.
fn write_todo(ctx: &TestContext, revs: &str) -> PathBuf {
    let path = ctx.dir.join("git-rebase-todo");
    let list: String = run(
        &ctx.dir,
        &["git", "log", "--reverse", "--format=pick %h %s", revs],
    );
    fs::write(&path, list).unwrap();
    path
}

/// The subject of each commit the list picks, in the file's order.
fn todo_subjects(ctx: &TestContext, todo: &Path) -> Vec<String> {
    fs::read_to_string(todo)
        .unwrap()
        .lines()
        .map(|line| {
            let (_action, rev) = line.split_once(' ').unwrap();
            run(&ctx.dir, &["git", "log", "-1", "--format=%s", rev])
                .trim()
                .to_string()
        })
        .collect()
}

fn sequence_editor_args(file: &Path) -> Args {
    Args {
        command: Some(Commands::SequenceEditor {
            file: file.to_path_buf(),
        }),
        ..Default::default()
    }
}

#[test]
fn sequence_editor_writes_the_edited_list() {
    let mut ctx = setup_todo(setup_clone!());
    let todo = write_todo(&ctx, "HEAD~3..HEAD");

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("<alt+j><enter>"));

    // The newest commit moved one place later in the rebase; git's file stays
    // oldest first.
    assert_eq!(
        todo_subjects(&ctx, &todo),
        ["add first-file", "add third-file", "add second-file"]
    );
    assert!(app.state.quit);
    assert_eq!(app.state.exit_code, 0, "git carries on with the list");
}

#[test]
fn sequence_editor_leaving_calls_the_rebase_off() {
    for key in ["q", "<esc>"] {
        let mut ctx = setup_todo(setup_clone!());
        let todo = write_todo(&ctx, "HEAD~3..HEAD");
        let before = fs::read_to_string(&todo).unwrap();

        let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
        ctx.update(&mut app, keys(&format!("<alt+j>{key}")));

        assert_eq!(fs::read_to_string(&todo).unwrap(), before, "list untouched");
        assert!(app.state.quit);
        assert_eq!(app.state.exit_code, 1, "{key}: non-zero tells git to stop");
    }
}

#[test]
fn sequence_editor_shows_the_list() {
    let mut ctx = setup_todo(setup_clone!());
    let todo = write_todo(&ctx, "HEAD~3..HEAD");

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("js"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn rebase_subcommand_opens_the_todo() {
    let mut ctx = setup_todo(setup_clone!());
    let args = Args {
        command: Some(Commands::Rebase {
            upstream: "HEAD~3".into(),
        }),
        ..Default::default()
    };

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), args);
    ctx.update(&mut app, keys(""));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn rebase_todo_shows_the_keys_on_request() {
    snapshot!(setup_todo(setup_clone!()), &format!("{OPEN_TODO}h"));
}

#[test]
fn rebase_interactive_asks_which_commit() {
    // Nothing under the cursor names a commit, so the log view asks for one.
    snapshot!(setup_todo(setup_clone!()), "ri");
}

#[test]
fn rebase_interactive_from_the_picked_commit() {
    snapshot!(setup_todo(setup_clone!()), "rij<enter>");
}

#[test]
fn rebase_interactive_quit_runs_nothing() {
    // Picked a commit, marked one to drop, then left: the log is as it was.
    snapshot!(setup_todo(setup_clone!()), "rij<enter>jdqll");
}
