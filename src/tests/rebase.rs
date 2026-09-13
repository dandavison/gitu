use super::*;
use crate::tests::log::{MULTI_ROW_FORMAT, with_log_renderer};

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
