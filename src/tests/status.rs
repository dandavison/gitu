//! Reaching the status screen from wherever gitu is.

use super::*;
use crate::git::diff::DiffType;

#[test]
fn status_from_the_log_view() {
    let mut ctx = with_renderer(setup_clone!());
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    ctx.update(&mut app, keys(STATUS));

    assert_eq!(app.state.screens.len(), 1);
    insta::assert_snapshot!(ctx.redact_buffer());
}

/// A patch gitu is showing as git's pager is the screen it was started on, so
/// status goes on top of it and quitting comes back to the patch.
#[test]
fn status_from_a_paged_patch() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "show", "HEAD"]);
    ctx.update(&mut app, keys(STATUS));

    assert_eq!(app.state.screens.len(), 2);
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("Unstaged changes"), "{buffer}");

    ctx.update(&mut app, keys("q"));
    assert_eq!(app.state.screens.len(), 1);
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("add firstfile"), "{buffer}");
}

#[test]
fn status_opens_on_the_first_unstaged_file() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a", "a\n");
    commit(&ctx.dir, "b", "b\n");
    fs::write(ctx.dir.join("a"), "staged\n").unwrap();
    run(&ctx.dir, &["git", "add", "a"]);
    fs::write(ctx.dir.join("b"), "unstaged\n").unwrap();
    fs::write(ctx.dir.join("untracked"), "").unwrap();
    ctx.config().general.status_cursor_on_changes = true;

    let (diff_type, header) = selected_file(&ctx.init_app());
    assert_eq!(diff_type, DiffType::WorkdirToIndex);
    assert!(header.starts_with("diff --git a/b b/b"), "{header}");
}

#[test]
fn status_with_nothing_unstaged_opens_on_the_first_staged_file() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a", "a\n");
    fs::write(ctx.dir.join("a"), "staged\n").unwrap();
    run(&ctx.dir, &["git", "add", "a"]);
    fs::write(ctx.dir.join("untracked"), "").unwrap();
    ctx.config().general.status_cursor_on_changes = true;

    let (diff_type, header) = selected_file(&ctx.init_app());
    assert_eq!(diff_type, DiffType::IndexToTree);
    assert!(header.starts_with("diff --git a/a b/a"), "{header}");
}

fn selected_file(app: &crate::app::App) -> (DiffType, String) {
    match &app.screen().get_selected_item().data {
        crate::item_data::ItemData::Delta { diff, file_i, .. } => (
            diff.diff_type.clone(),
            diff.file_diff_header(*file_i).to_string(),
        ),
        data => panic!("not a file: {data:?}"),
    }
}

/// A file a refresh brings that the view has not had before starts folded, as
/// the view's first files did.
#[test]
fn a_file_new_to_a_refresh_starts_folded() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "b.rs", "beta\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    let mut app = ctx.init_app();

    fs::write(ctx.dir.join("b.rs"), "beta changed\n").unwrap();
    ctx.update(&mut app, keys(REFRESH));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("b.rs"), "{buffer}");
    assert!(!buffer.contains("beta changed"), "{buffer}");
}

#[test]
fn a_file_first_staged_is_folded_as_it_was_unstaged() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "b.rs", "beta\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    fs::write(ctx.dir.join("b.rs"), "beta changed\n").unwrap();
    ctx.config().general.status_cursor_on_changes = true;
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("<tab>"));

    run(&ctx.dir, &["git", "add", "a.rs", "b.rs"]);
    ctx.update(&mut app, keys(REFRESH));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("Staged changes"), "{buffer}");
    assert!(buffer.contains("alpha changed"), "{buffer}");
    assert!(buffer.contains("b.rs"), "{buffer}");
    assert!(!buffer.contains("beta changed"), "{buffer}");
}

#[test]
fn a_folded_file_stays_folded_when_it_changes() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    let mut app = ctx.init_app();
    assert!(!ctx.redact_buffer().contains("alpha changed"));

    fs::write(ctx.dir.join("a.rs"), "alpha changed again\n").unwrap();
    ctx.update(&mut app, keys(REFRESH));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs"), "{buffer}");
    assert!(!buffer.contains("alpha changed"), "{buffer}");
}

#[test]
fn an_unfolded_file_stays_unfolded_when_it_changes() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    ctx.config().general.status_cursor_on_changes = true;
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("<tab>"));
    assert!(ctx.redact_buffer().contains("alpha changed"));

    fs::write(ctx.dir.join("a.rs"), "alpha changed again\n").unwrap();
    ctx.update(&mut app, keys(REFRESH));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha changed again"), "{buffer}");
}
