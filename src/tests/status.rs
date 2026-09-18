//! Reaching the status screen from wherever gitu is.

use super::*;

#[test]
fn status_from_the_log_view() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    ctx.update(&mut app, keys(STATUS));

    assert_eq!(app.state.screens.len(), 1);
    insta::assert_snapshot!(ctx.redact_buffer());
}

/// A patch gitu is showing as git's pager is the screen it was started on, so
/// status goes on top of it and quitting comes back to the patch.
#[test]
fn status_from_a_paged_patch() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "show", "HEAD"]);
    ctx.update(&mut app, keys(STATUS));

    assert_eq!(app.state.screens.len(), 2);
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("Unstaged changes"), "{buffer}");

    ctx.update(&mut app, keys("q"));
    assert_eq!(app.state.screens.len(), 1);
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("add firstfile"), "{buffer}");
}
