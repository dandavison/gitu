use super::*;

fn setup_scroll(mut ctx: TestContext) -> (TestContext, crate::app::App) {
    for file in ["file-1", "file-2", "file-3"] {
        commit(&ctx.dir, file, "");
        fs::write(
            ctx.dir.join(file),
            (1..=20).fold(String::new(), |mut acc, i| {
                use std::fmt::Write as _;

                writeln!(acc, "line {} ({})", i, file).unwrap();
                acc
            }),
        )
        .unwrap();
    }

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("jjjj<tab>k<tab>k<tab>"));
    (ctx, app)
}

#[test]
fn scroll_down() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    ctx.update(&mut app, keys("<ctrl+d>"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn scroll_past_selection() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    ctx.update(&mut app, keys("<ctrl+d><ctrl+d><ctrl+d>"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn move_prev_sibling() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    ctx.update(&mut app, keys("<alt+k><alt+k>"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn move_next_sibling() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    ctx.update(&mut app, keys("<alt+j>"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn move_next_then_parent_section() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    ctx.update(&mut app, keys("<alt+j><alt+h>"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn exit_from_picker_exits_menu() {
    snapshot!(setup_clone!(), "bb<esc>");
}

#[test]
fn re_enter_picker_from_menu() {
    snapshot!(setup_clone!(), "bb<esc>bb");
}

#[test]
fn renderer_features_offers_the_configured_ones() {
    snapshot!(setup_clone!(), "|");
}

#[test]
fn choosing_a_renderer_feature_marks_it() {
    // Picking `side-by-side` turns it on; re-opening shows it marked, and
    // picking it again turns it back off.
    let ctx = setup_clone!();
    let mut ctx = ctx;
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("|<enter>"));
    assert_eq!(&*app.state.features, ["side-by-side".to_string()]);

    ctx.update(&mut app, keys("|<enter>"));
    assert!(
        app.state.features.is_empty(),
        "picking it again turns it off"
    );
}
