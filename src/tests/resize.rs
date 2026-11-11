//! A change of the terminal's size renders every screen again at the new width: delta lays out
//! side-by-side panels and decorations, and decides `side-by-side = <min-width>`, by the width it
//! is asked to render at.

use super::*;
use std::path::Path;

/// The test renderer, which also writes the width it is asked to render at (`COLUMNS`) after each
/// file's name, and, if `log` is given, writes that width to `log` each time it runs.
fn width_reporting_renderer(log: Option<&Path>) -> Vec<String> {
    let file_header = r#"label,suffix;header=0}"#;
    let with_width = r#"label,suffix " @" ENVIRON["COLUMNS"];header=0}"#;
    super::pager::annotating_renderer()
        .into_iter()
        .map(|arg| {
            if !arg.contains(file_header) {
                return arg;
            }
            let arg = arg.replace(file_header, with_width);
            match log {
                Some(log) => arg.replacen(
                    "BEGIN{",
                    &format!(r#"BEGIN{{print ENVIRON["COLUMNS"] >> "{}";"#, log.display()),
                    1,
                ),
                None => arg,
            }
        })
        .collect()
}

fn with_a_change(mut ctx: TestContext, log: Option<&Path>) -> TestContext {
    ctx.config().general.diff_renderer.command = width_reporting_renderer(log);
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx
}

#[test]
fn a_resize_renders_the_pager_view_again_at_the_new_width() {
    let mut ctx = with_a_change(setup_clone!(), None);
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    let before = ctx.redact_buffer();
    assert!(before.contains("firstfile @78"), "{before}");

    ctx.resize(&mut app, &[(120, 20)]);
    let after = ctx.redact_buffer();
    assert!(after.contains("firstfile @118"), "{after}");
}

#[test]
fn a_resize_renders_the_status_view_again_at_the_new_width() {
    let mut ctx = with_a_change(setup_clone!(), None);
    let mut app = ctx.init_app();
    let before = ctx.redact_buffer();
    assert!(before.contains("firstfile @78"), "{before}");

    ctx.resize(&mut app, &[(120, 20)]);
    let after = ctx.redact_buffer();
    assert!(after.contains("firstfile @118"), "{after}");
}

/// Dragging a window's edge sends many resizes in a row. delta runs again once, at the last size,
/// and not once for each resize.
#[test]
fn many_resizes_in_a_row_render_once_at_the_last_width() {
    let ctx = setup_clone!();
    let log = ctx.dir.join(".git").join("render-widths");
    let mut ctx = with_a_change(ctx, Some(&log));
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    let widths = || fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(widths(), "78\n");

    ctx.resize(&mut app, &[(90, 20), (100, 20), (110, 20), (120, 20)]);
    assert_eq!(widths(), "78\n118\n");
    let screen = ctx.redact_buffer();
    assert!(screen.contains("firstfile @118"), "{screen}");
}
