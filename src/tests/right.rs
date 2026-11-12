use super::*;
use crate::app::App;
use crate::cmd_log::CmdLogEntry;

/// Whether the last key ran the editor, which the tests set to `true`.
fn edited(app: &App) -> bool {
    app.state.current_cmd_log.entries.iter().any(|entry| {
        matches!(&*entry.read().unwrap(), CmdLogEntry::Cmd { args, .. } if args.starts_with("true "))
    })
}

fn on_a_folded_file(mut ctx: TestContext) -> (TestContext, App) {
    commit(&ctx.dir, "a", "a\n");
    fs::write(ctx.dir.join("a"), "changed\n").unwrap();
    ctx.config().general.status_cursor_on_changes = true;
    let app = ctx.init_app();
    (ctx, app)
}

#[test]
fn right_unfolds_a_folded_file() {
    let (mut ctx, mut app) = on_a_folded_file(setup_clone!());
    assert!(
        !ctx.redact_buffer().contains("changed"),
        "{}",
        ctx.redact_buffer()
    );

    ctx.update(&mut app, keys("<right>"));
    assert!(
        ctx.redact_buffer().contains("changed"),
        "{}",
        ctx.redact_buffer()
    );
    assert!(!edited(&app));
}

#[test]
fn right_on_an_unfolded_file_stays_in_gitu() {
    let (mut ctx, mut app) = on_a_folded_file(setup_clone!());
    ctx.update(&mut app, keys("<tab><right>"));
    assert!(!edited(&app));
    assert!(
        ctx.redact_buffer().contains("changed"),
        "{}",
        ctx.redact_buffer()
    );
}

#[test]
fn right_on_a_hunk_stays_in_gitu() {
    let (mut ctx, mut app) = on_a_folded_file(setup_clone!());
    ctx.update(&mut app, keys("<tab>j<right>"));
    assert!(!edited(&app));
}

#[test]
fn enter_on_a_file_opens_the_editor() {
    let (mut ctx, mut app) = on_a_folded_file(setup_clone!());
    ctx.update(&mut app, keys("<enter>"));
    assert!(edited(&app));
}
