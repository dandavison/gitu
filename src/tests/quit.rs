use super::*;

#[test]
pub(crate) fn quit() {
    let app = snapshot!(setup_clone!(), "q");
    assert!(app.state.quit);
}

#[test]
pub(crate) fn quit_from_menu() {
    let app = snapshot!(setup_clone!(), "hq");
    assert!(!app.state.quit);
}

#[test]
pub(crate) fn ctrl_c_quits() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("<ctrl+c>"));
    assert!(app.state.quit);
}

#[test]
pub(crate) fn ctrl_c_quits_from_menu() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("h<ctrl+c>"));
    assert!(app.state.quit);
}

#[test]
pub(crate) fn confirm_quit_prompt() {
    let mut ctx = setup_clone!();
    ctx.config().general.confirm_quit.enabled = true;

    let app = snapshot!(ctx, "q");
    assert!(!app.state.quit);
}

#[test]
pub(crate) fn confirm_quit() {
    let mut ctx = setup_clone!();
    ctx.config().general.confirm_quit.enabled = true;

    let app = snapshot!(ctx, "qy");
    assert!(app.state.quit);
}
