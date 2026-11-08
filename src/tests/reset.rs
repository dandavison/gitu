use super::*;

fn setup(ctx: TestContext) -> TestContext {
    commit(&ctx.dir, "unwanted-file", "");
    ctx
}

#[test]
pub(crate) fn reset_menu() {
    snapshot!(setup(setup_clone!()), "lljX");
}

#[test]
pub(crate) fn reset_soft_prompt() {
    snapshot!(setup(setup_clone!()), "lljXsq");
}

#[test]
pub(crate) fn reset_soft() {
    snapshot!(setup(setup_clone!()), "lljXs<enter>q");
}

#[test]
pub(crate) fn reset_mixed() {
    snapshot!(setup(setup_clone!()), "lljXm<enter>q");
}

#[test]
fn reset_hard() {
    snapshot!(setup(setup_clone!()), "lljXh<enter>q");
}

/// Resetting to the commit selected in the log leaves it selected: it is now
/// HEAD, at the top of the log.
#[test]
fn reset_hard_in_log_keeps_the_reset_commit_selected() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a", "");
    commit(&ctx.dir, "b", "");
    commit(&ctx.dir, "c", "");
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("llj"));
    assert_eq!("add b", selected_summary(&app));

    ctx.update(&mut app, keys("Xh<enter>"));
    assert_eq!("add b", selected_summary(&app), "{}", ctx.redact_buffer());
}

fn selected_summary(app: &crate::app::App) -> String {
    match &app.state.screens.last().unwrap().get_selected_item().data {
        crate::item_data::ItemData::Commit { summary, .. } => summary.clone(),
        data => panic!("not a commit: {data:?}"),
    }
}
