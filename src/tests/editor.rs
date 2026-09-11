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

/// One key folds the whole view down to its headings, and opens it again.
#[test]
fn folding_everything_and_opening_it_again() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());

    ctx.update(&mut app, keys("<backtab>"));
    let folded = ctx.redact_buffer();
    ctx.update(&mut app, keys("<backtab>"));
    let opened = ctx.redact_buffer();

    assert!(!folded.contains("file-1"), "still open:\n{folded}");
    assert!(folded.contains("Unstaged changes"), "{folded}");
    assert!(
        opened.contains("line 1 (file-1)"),
        "still folded:\n{opened}"
    );
}

/// A page is a whole viewport, which is two half pages.
#[test]
fn a_page_is_a_whole_viewport() {
    let (mut halves, mut halves_app) = setup_scroll(TestContext::setup_clone("page_in_halves"));
    halves.update(&mut halves_app, keys("<ctrl+d><ctrl+d>"));

    let (mut page, mut page_app) = setup_scroll(TestContext::setup_clone("page_at_once"));
    page.update(&mut page_app, keys("<pagedown>"));

    assert_eq!(page.redact_buffer(), halves.redact_buffer());
}

#[test]
fn a_page_down_and_a_page_up_come_back() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    let before = ctx.redact_buffer();

    ctx.update(&mut app, keys("<pagedown>"));
    let paged = ctx.redact_buffer();
    ctx.update(&mut app, keys("<pageup>"));

    assert_ne!(paged, before, "page down did not scroll");
    assert_eq!(ctx.redact_buffer(), before, "page up did not come back");
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

/// The features the user defined for themselves are theirs to choose from too,
/// without their having to name them to gitu as well.
#[test]
fn renderer_features_offers_those_defined_in_git_config() {
    let mut ctx = setup_clone!();
    run(
        &ctx.dir,
        &["git", "config", "delta.my-theme.syntax-theme", "Nord"],
    );
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("|my-theme<enter>"));
    assert_eq!(&*app.state.features, ["my-theme".to_string()]);
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
