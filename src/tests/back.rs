use super::*;

#[test]
fn left_goes_back_to_the_screen_before() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll<right>"));
    assert_eq!(app.state.screens.len(), 3);

    ctx.update(&mut app, keys("<left>"));
    assert_eq!(app.state.screens.len(), 2);
}

/// Going back from the first screen would leave gitu, which left is not
/// expected to do.
#[test]
fn left_on_the_first_screen_stays() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("<left>"));
    assert!(!app.state.quit);
    assert_eq!(app.state.screens.len(), 1);
}

#[test]
fn left_closes_a_menu() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll<right>"));
    let closed = app.state.pending_menu.as_ref().map(|m| m.menu);

    ctx.update(&mut app, keys("b<left>"));
    assert_eq!(app.state.pending_menu.as_ref().map(|m| m.menu), closed);
    assert_eq!(app.state.screens.len(), 3);
}
