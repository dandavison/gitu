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
    let mut ctx = setup_todo(setup_clone!());
    let todo = write_todo(&ctx, "HEAD~3..HEAD");
    let before = fs::read_to_string(&todo).unwrap();

    let mut app = ctx.init_app_with_args(ctx.dir.clone(), sequence_editor_args(&todo));
    ctx.update(&mut app, keys("<alt+j>q"));

    assert_eq!(fs::read_to_string(&todo).unwrap(), before, "list untouched");
    assert!(app.state.quit);
    assert_eq!(app.state.exit_code, 1, "non-zero tells git to abort");
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
