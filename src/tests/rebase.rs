use super::*;
use crate::cli::{Args, Commands};
use crate::tests::log::{MULTI_ROW_FORMAT, with_log_renderer};
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
    ctx
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

#[test]
fn rebase_todo_mark_leaves_the_view_where_it_is() {
    let mut ctx = setup_clone!();
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
fn rebase_todo_shows_the_log_renderer_rows() {
    // The commits look as they do in the log view, with the instruction in front.
    snapshot!(
        with_log_renderer(setup_todo(setup_clone!()), &[], MULTI_ROW_FORMAT),
        &format!("{OPEN_TODO}js")
    );
}

#[test]
fn rebase_todo_redraws_moved_renderer_hyperlinks() {
    let mut ctx = setup_todo(setup_clone!());
    let config = ctx.config();
    config.general.log_renderer.enabled = true;
    config.general.log_renderer.command = vec![
        "git".into(),
        "log".into(),
        "--color=always".into(),
        "--format={commit}%n▸ \x1b]8;;https://example.com/%H\x1b\\%h\x1b]8;;\x1b\\ %s".into(),
    ];

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
fn sequence_editor_leaving_hands_git_back_its_list() {
    // One `q`, and git carries on with the list it wrote: nothing gitu did is
    // handed back, and git isn't told anything went wrong.
    let mut ctx = setup_todo(setup_clone!());
    let todo = write_todo(&ctx, "HEAD~3..HEAD");
    let before = fs::read_to_string(&todo).unwrap();

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("<alt+j>q"));

    assert_eq!(fs::read_to_string(&todo).unwrap(), before, "list untouched");
    assert!(app.state.quit);
    assert_eq!(app.state.exit_code, 0, "git has nothing to complain about");
}

#[test]
fn sequence_editor_abort_calls_the_rebase_off() {
    let mut ctx = setup_todo(setup_clone!());
    let todo = write_todo(&ctx, "HEAD~3..HEAD");
    let before = fs::read_to_string(&todo).unwrap();

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("Q"));

    assert_eq!(fs::read_to_string(&todo).unwrap(), before, "list untouched");
    assert!(app.state.quit);
    assert_eq!(app.state.exit_code, 1, "non-zero tells git to stop");
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
