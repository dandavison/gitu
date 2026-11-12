//! git commands the user defines in config and binds to keys, opened as views.

use super::*;

/// `ctx` with `command` defined as the view `name`, bound to `key`.
fn with_view(mut ctx: TestContext, name: &str, command: &str, key: char) -> TestContext {
    let config = ctx.config();
    config
        .general
        .views
        .insert(name.to_owned(), command.to_owned());
    config
        .general
        .diff_renderer
        .feature_keys
        .insert(name.to_owned(), key);
    ctx
}

/// A view's key opens its git command, filled in from the row under the cursor,
/// on top of the screen it was pressed on; `q` goes back.
#[test]
fn a_view_key_opens_the_git_command_on_the_commit_under_the_cursor() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "a", "");
    let mut ctx = with_view(ctx, "subject", "git show --format=viewed-%s {commit}", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("llo"));
    assert!(
        ctx.redact_buffer().contains("viewed-add a"),
        "{}",
        ctx.redact_buffer()
    );
    let screens = app.state.screens.len();

    ctx.update(&mut app, keys("q"));
    assert_eq!(app.state.screens.len(), screens - 1);
}

/// A view is a git command whose output gitu shows: `git diff`, `git show` or
/// `git log`. Anything else is refused, and nothing is opened.
#[test]
fn a_view_that_is_not_git_diff_show_or_log_is_refused() {
    let ctx = setup_clone!();
    let mut ctx = with_view(ctx, "listing", "ls", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("o"));

    assert_eq!(app.state.screens.len(), 1);
    assert!(
        ctx.redact_buffer().contains("listing"),
        "{}",
        ctx.redact_buffer()
    );
}

/// A view asking for what is not under the cursor is not opened, and says why.
#[test]
fn a_view_needing_a_commit_is_not_opened_off_one() {
    let ctx = setup_clone!();
    let mut ctx = with_view(ctx, "subject", "git show {commit}", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("o"));

    assert_eq!(app.state.screens.len(), 1);
    assert!(
        ctx.redact_buffer().contains("{commit}"),
        "{}",
        ctx.redact_buffer()
    );
}
