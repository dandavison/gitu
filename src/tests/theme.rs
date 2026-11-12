use super::*;
use crate::config::ColorMode;
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
    after_in(function_name!(), None, keys_).0
}

/// What the screen is rendered with and what is on it after `keys_`, with
/// delta in `mode`, in a repository named for `test` and the rest.
fn after_in(test: &str, mode: Option<ColorMode>, keys_: &str) -> (Vec<String>, String) {
    let name: String = format!("{test}_{mode:?}_{keys_}")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut ctx = TestContext::setup_clone(&format!("theme_after_{name}"));
    with_themes(&ctx);
    ctx.config().general.diff_renderer.color_mode = mode;
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(keys_));
    (rendered_with(&app), ctx.redact_buffer())
}

/// delta has settled on light or dark before gitu starts, and the arrows move
/// through its themes for that mode only.
#[test]
fn the_arrows_cycle_through_the_themes_for_the_mode_delta_is_in() {
    let light = Some(ColorMode::Light);
    assert_eq!(after_in(function_name!(), light, "T<down>").0, ["theme-b"]);
    assert_eq!(
        after_in(function_name!(), light, "T<down><down>").0,
        Vec::<String>::new()
    );

    let dark = Some(ColorMode::Dark);
    assert_eq!(after_in(function_name!(), dark, "T<down>").0, ["theme-a"]);
    assert_eq!(
        after_in(function_name!(), dark, "T<down><down>").0,
        Vec::<String>::new()
    );
}

/// The list has every theme, those for the other mode last, saying which mode
/// they are for.
#[test]
fn the_list_has_the_themes_for_the_other_mode_too() {
    let light = Some(ColorMode::Light);
    let (rendered, screen) = after_in(function_name!(), light, "T<ctrl+l><down><down>");
    assert_eq!(rendered, ["theme-a"]);
    assert!(screen.contains("theme-a (dark)"), "{screen}");
}

/// One of them chosen from the list stays in view on going back to one row,
/// from where the arrows move through the themes for delta's mode again.
#[test]
fn a_theme_for_the_other_mode_stays_on_going_back_to_one_row() {
    let light = Some(ColorMode::Light);
    let (rendered, screen) = after_in(function_name!(), light, "T<ctrl+l><down><down><ctrl+l>");
    assert_eq!(rendered, ["theme-a"]);
    assert!(screen.contains("Theme theme-a (dark)"), "{screen}");

    let (rendered, _) = after_in(function_name!(), light, "T<ctrl+l><down><down><ctrl+l><up>");
    assert_eq!(rendered, ["theme-b"]);
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

/// The themes are those the renderer finds in git config, which may not be those gitu finds:
/// offered a theme the renderer cannot find, the user would see the selector change theme while
/// the diff does not.
#[test]
fn theme_mode_offers_only_the_themes_the_renderer_finds() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    let empty = ctx.dir.join(".git").join("renderer-gitconfig");
    fs::write(&empty, "").unwrap();
    ctx.config().general.diff_renderer.git_config =
        crate::config::RendererGitConfig::new(move |_| git2::Config::open(&empty).ok());
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T"));
    assert!(app.state.picker.is_none());
    assert!(
        ctx.redact_buffer().contains("No themes are defined"),
        "{}",
        ctx.redact_buffer()
    );
}

/// A digit renders with the theme of that number at once, so that two can be
/// compared by switching back and forth. 0 is no theme.
#[test]
fn a_digit_renders_with_the_theme_of_that_number() {
    assert_eq!(rendered_after("T2"), ["theme-b"]);
    assert_eq!(rendered_after("T21"), ["theme-a"]);
    assert_eq!(rendered_after("T210"), Vec::<String>::new());
}

/// A feature key toggles its feature in theme mode, since nothing is typed in
/// its one row, so that themes can be judged side by side and not.
#[test]
fn a_feature_key_toggles_its_feature_in_theme_mode() {
    assert_eq!(rendered_after("T1S"), ["side-by-side", "theme-a"]);
    assert_eq!(rendered_after("T1SS"), ["theme-a"]);
    assert_eq!(rendered_after("T1S2"), ["side-by-side", "theme-b"]);
}

/// A feature toggled in theme mode stays as it is when theme mode is left.
#[test]
fn a_feature_toggled_in_theme_mode_stays_toggled() {
    assert_eq!(rendered_after("TS<esc>"), ["side-by-side"]);
}

/// A digit with no theme of that number does nothing.
#[test]
fn a_digit_past_the_last_theme_does_nothing() {
    assert_eq!(rendered_after("T23"), ["theme-b"]);
}

/// The digits number the themes the arrows move through: those for delta's
/// mode.
#[test]
fn the_digits_number_the_themes_for_the_mode_delta_is_in() {
    let light = Some(ColorMode::Light);
    assert_eq!(after_in(function_name!(), light, "T1").0, ["theme-b"]);
    assert_eq!(after_in(function_name!(), light, "T12").0, ["theme-b"]);
}

/// Each theme's number is the digit that selects it.
#[test]
fn one_row_shows_the_themes_number() {
    let ctx = setup_clone!();
    with_themes(&ctx);
    snapshot!(ctx, "T2");
}

/// In the list, digits are typed, since theme names have them.
#[test]
fn a_digit_in_the_list_searches() {
    let mut ctx = setup_clone!();
    with_themes(&ctx);
    run(&ctx.dir, &["git", "config", "delta.theme-2.dark", "true"]);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<ctrl+l>2<enter>"));
    assert_eq!(theme(&app), Some("theme-2"));
}

/// A builtin delta feature that changes how a diff looks, such as
/// diff-so-fancy, is offered after the themes in git config when the config
/// lists it, for either mode, since it sets neither dark nor light.
#[test]
fn builtin_themes_are_offered_after_the_themes_in_git_config() {
    let after = |mode, keys_: &str| {
        let name: String = format!("builtin_themes_{mode:?}_{keys_}")
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let mut ctx = TestContext::setup_clone(&name);
        with_themes(&ctx);
        let renderer = &mut ctx.config().general.diff_renderer;
        renderer.color_mode = mode;
        renderer.builtin_themes = vec!["diff-so-fancy".into()];
        let mut app = ctx.init_app();
        ctx.update(&mut app, keys(keys_));
        rendered_with(&app)
    };
    assert_eq!(after(None, "T<down><down><down>"), ["diff-so-fancy"]);
    assert_eq!(
        after(None, "T<down><down><down><down>"),
        Vec::<String>::new()
    );
    assert_eq!(
        after(Some(ColorMode::Light), "T<down><down>"),
        ["diff-so-fancy"]
    );
    assert_eq!(
        after(Some(ColorMode::Dark), "T<down><down>"),
        ["diff-so-fancy"]
    );
}
