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
    ctx.update(&mut app, keys(HALF_PAGE_DOWN));
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

/// A terminal sends shift-tab as backtab with the shift modifier still set.
/// The keycode already says shift was held, so a `backtab` binding — which is
/// what config can name — has to match it.
#[test]
fn shift_tab_reaches_a_backtab_binding() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());

    ctx.update(&mut app, keys("<shift+backtab>"));

    assert!(
        !ctx.redact_buffer().contains("file-1"),
        "shift-tab did nothing:\n{}",
        ctx.redact_buffer()
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

/// Space is how every pager since `more` turns the page, and what magit binds
/// it to; backspace is its other half.
#[test]
fn space_turns_the_page() {
    let (mut keys_ctx, mut keys_app) = setup_scroll(TestContext::setup_clone("page_by_key"));
    keys_ctx.update(&mut keys_app, keys("<pagedown><pagedown><pageup>"));

    let (mut space_ctx, mut space_app) = setup_scroll(TestContext::setup_clone("page_by_space"));
    space_ctx.update(&mut space_app, keys("<space><space><backspace>"));

    assert_eq!(space_ctx.redact_buffer(), keys_ctx.redact_buffer());
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

/// Paging through a diff carries the cursor with the view, selecting whatever
/// row it lands on — a line within a hunk, when that is what is there.
#[test]
fn paging_carries_the_cursor_into_the_hunk() {
    let (mut ctx, mut app) = setup_scroll(setup_clone!());
    ctx.update(
        &mut app,
        keys(&format!("{HALF_PAGE_DOWN}{HALF_PAGE_DOWN}{HALF_PAGE_DOWN}")),
    );
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
