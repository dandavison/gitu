use super::*;
use std::path::{Path, PathBuf};

/// Two themes, as delta defines one: a feature that sets `dark` or `light`. A
/// feature that sets neither is not a theme.
fn with_themes(ctx: &TestContext) -> PathBuf {
    for (key, value) in [
        ("delta.theme-a.dark", "true"),
        ("delta.theme-b.light", "true"),
        ("delta.not-a-theme.syntax-theme", "Nord"),
        // The repository's own git config holds the `delta-tui` section, so it
        // is where a theme chosen is written.
        ("delta-tui.visit-context-lines", "true"),
    ] {
        run(&ctx.dir, &["git", "config", key, value]);
    }
    ctx.dir.join(".git/config")
}

/// The theme gitu would read from the git config file at `path`.
fn saved_theme(path: &Path) -> Option<String> {
    let git_config = git2::Config::open(path).unwrap();
    crate::config::init_config_from(PathBuf::from("/nonexistent/config.toml"), Some(&git_config))
        .unwrap()
        .general
        .diff_renderer
        .theme
}

fn theme(app: &crate::app::App) -> Option<&str> {
    app.state.theme.as_deref()
}

/// The features the screen is rendered with.
fn rendered_with(app: &crate::app::App) -> Vec<String> {
    app.screen().features.to_vec()
}

/// Theme mode is one row at the bottom, showing the theme on screen, so that
/// what is being judged stays in view.
#[test]
fn theme_mode_shows_only_the_current_theme() {
    let ctx = setup_clone!();
    with_themes(&ctx);
    snapshot!(ctx, "T<down>");
}

/// Moving to a theme renders the screen with it, before anything is chosen.
#[test]
fn moving_to_a_theme_renders_with_it() {
    assert_eq!(rendered_after("T<down>"), ["theme-a"]);
}

/// The themes are the features that set `dark` or `light`, after the user's
/// own configuration with no theme overlaid, and the list wraps around.
#[test]
fn the_arrows_cycle_through_the_themes() {
    assert_eq!(rendered_after("T<down><down>"), ["theme-b"]);
    assert_eq!(rendered_after("T<down><down><down>"), Vec::<String>::new());
    assert_eq!(rendered_after("T<down><down><down><up>"), ["theme-b"]);
}

/// What the screen is rendered with after `keys_`, which leave theme mode open.
/// In a test, theme mode ends when the keys run out, so what it shows is
/// observed with all the keys that lead to it given at once.
fn rendered_after(keys_: &str) -> Vec<String> {
    let name: String = keys_
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut ctx = TestContext::setup_clone(&format!("rendered_after_{name}"));
    with_themes(&ctx);
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(keys_));
    rendered_with(&app)
}

/// Enter keeps the theme, and saves it to the git config that holds the
/// `delta-tui` section, beside what was already there.
#[test]
fn enter_chooses_the_theme_and_saves_it() {
    let mut ctx = setup_clone!();
    let path = with_themes(&ctx);
    let mine = fs::read_to_string(&path).unwrap();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<down><enter>"));

    assert_eq!(theme(&app), Some("theme-a"));
    assert_eq!(rendered_with(&app), ["theme-a"]);
    assert!(app.state.picker.is_none(), "theme mode is over");
    assert!(fs::read_to_string(&path).unwrap().starts_with(&mine));
    assert_eq!(saved_theme(&path).as_deref(), Some("theme-a"));
}

/// q leaves theme mode with the theme it was entered with, and saves nothing.
#[test]
fn q_restores_the_theme() {
    let mut ctx = setup_clone!();
    let path = with_themes(&ctx);
    ctx.config().general.diff_renderer.theme = Some("theme-b".into());
    let mine = fs::read_to_string(&path).unwrap();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<up>q"));
    assert_eq!(theme(&app), Some("theme-b"));
    assert_eq!(rendered_with(&app), ["theme-b"]);
    assert!(app.state.picker.is_none(), "theme mode is over");
    assert_eq!(fs::read_to_string(&path).unwrap(), mine);
}

#[test]
fn escape_restores_the_theme() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<down><esc>"));
    assert_eq!(theme(&app), None);
    assert_eq!(rendered_with(&app), Vec::<String>::new());
}

/// Theme mode opens on the theme in force.
#[test]
fn theme_mode_starts_at_the_current_theme() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    ctx.config().general.diff_renderer.theme = Some("theme-a".into());
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<down>"));
    assert_eq!(rendered_with(&app), ["theme-b"]);
}

/// A theme saved in config is in force from the start.
#[test]
fn the_configured_theme_renders_from_the_start() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    ctx.config().general.diff_renderer.theme = Some("theme-b".into());
    let app = ctx.init_app();

    assert_eq!(theme(&app), Some("theme-b"));
    assert_eq!(rendered_with(&app), ["theme-b"]);
}

/// Choosing no theme removes the saved one: the user's own configuration
/// applies untouched.
#[test]
fn choosing_no_theme_unsets_the_saved_one() {
    let mut ctx = setup_clone!();
    let path = with_themes(&ctx);
    run(
        &ctx.dir,
        &[
            "git",
            "config",
            "delta-tui.general.diff-renderer.theme",
            "theme-a",
        ],
    );
    ctx.config().general.diff_renderer.theme = Some("theme-a".into());
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<up><enter>"));

    assert_eq!(theme(&app), None);
    assert_eq!(rendered_with(&app), Vec::<String>::new());
    assert_eq!(saved_theme(&path), None);
}

/// ctrl+l shows every theme, where typing searches them, and back.
#[test]
fn ctrl_l_lists_every_theme() {
    let ctx = setup_clone!();
    with_themes(&ctx);
    snapshot!(ctx, "T<ctrl+l>");
}

#[test]
fn typing_in_the_list_searches_the_themes() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<ctrl+l>b<enter>"));
    assert_eq!(theme(&app), Some("theme-b"));
}

#[test]
fn ctrl_l_in_the_list_goes_back_to_one_row() {
    let ctx = setup_clone!();
    with_themes(&ctx);
    snapshot!(ctx, "T<ctrl+l><down><ctrl+l>");
}

/// Themes are chosen one at a time in theme mode, so the feature list, which
/// turns any number on, leaves them out.
#[test]
fn the_feature_list_leaves_themes_out() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    ctx.config().general.diff_renderer.features = vec!["*".into()];
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("-"));
    let screen = ctx.redact_buffer();
    assert!(screen.contains("not-a-theme"), "{screen}");
    assert!(!screen.contains("theme-a"), "{screen}");
}

/// The theme is overlaid last, so that it has its way over a feature that sets
/// the same keys, whenever the feature was turned on.
#[test]
fn the_theme_is_overlaid_after_the_features() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    ctx.config().general.diff_renderer.theme = Some("theme-a".into());
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("S"));
    assert_eq!(rendered_with(&app), ["side-by-side", "theme-a"]);
}

/// With no theme defined there is nothing to switch between.
#[test]
fn theme_mode_without_themes_says_so() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T"));
    assert!(app.state.picker.is_none());
    assert!(
        ctx.redact_buffer().contains("No themes are defined"),
        "{}",
        ctx.redact_buffer()
    );
}
