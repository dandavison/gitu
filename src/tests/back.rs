use super::*;
use crate::item_data::ItemData;

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

/// What right unfolded, left folds again, without leaving the screen.
#[test]
fn left_folds_what_right_unfolded_in_a_show_screen() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll<right><tab><right>"));
    let screen = app.screen();
    assert!(!screen.is_collapsed(screen.get_selected_item()));

    ctx.update(&mut app, keys("<left>"));
    assert_eq!(app.state.screens.len(), 3);
    let screen = app.screen();
    assert!(screen.is_collapsed(screen.get_selected_item()));
}

#[test]
fn left_folds_a_file_right_unfolded_in_status() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a", "a\n");
    fs::write(ctx.dir.join("a"), "changed\n").unwrap();
    ctx.config().general.status_cursor_on_changes = true;
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("<right><left>"));
    assert!(
        !ctx.redact_buffer().contains("changed"),
        "{}",
        ctx.redact_buffer()
    );
}

/// A show screen opens with its hunks unfolded; left goes back from there
/// rather than folding them.
#[test]
fn left_goes_back_from_a_hunk_the_screen_opened_unfolded() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll<right>"));
    let screen = app.screen();
    assert!(screen.get_selected_item().data.is_section());
    assert!(!screen.is_collapsed(screen.get_selected_item()));

    ctx.update(&mut app, keys("<left>"));
    assert_eq!(app.state.screens.len(), 2);
}

/// Left folds what right unfolded from anywhere inside it, moving the cursor
/// back onto it.
#[test]
fn left_from_inside_what_right_unfolded_folds_it() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll<right><alt+left><tab><right><down>"));
    let screen = app.screen();
    assert!(matches!(
        screen.get_selected_item().data,
        ItemData::Hunk { .. }
    ));

    ctx.update(&mut app, keys("<left>"));
    assert_eq!(app.state.screens.len(), 3);
    let screen = app.screen();
    assert!(matches!(
        screen.get_selected_item().data,
        ItemData::Delta { .. }
    ));
    assert!(screen.is_collapsed(screen.get_selected_item()));
}

#[test]
fn left_from_a_line_of_a_file_right_unfolded_folds_the_file() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a", "a\n");
    fs::write(ctx.dir.join("a"), "changed\n").unwrap();
    ctx.config().general.status_cursor_on_changes = true;
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("<right><down><down>"));
    assert!(matches!(
        app.screen().get_selected_item().data,
        ItemData::HunkLine { .. }
    ));

    ctx.update(&mut app, keys("<left>"));
    assert!(matches!(
        app.screen().get_selected_item().data,
        ItemData::Delta { .. }
    ));
    assert!(
        !ctx.redact_buffer().contains("changed"),
        "{}",
        ctx.redact_buffer()
    );
}
