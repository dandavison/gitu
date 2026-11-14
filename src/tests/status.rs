//! Reaching the status screen from wherever gitu is.

use super::pager::draws_lines_of;
use super::*;
use crate::git::diff::DiffType;
use std::time::{Duration, Instant};

#[test]
fn status_from_the_log_view() {
    let mut ctx = setup_clone!();
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

#[test]
fn folding_everything_folds_the_files_not_the_sections() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "b.rs", "beta\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    fs::write(ctx.dir.join("b.rs"), "beta changed\n").unwrap();
    run(&ctx.dir, &["git", "add", "b.rs"]);
    ctx.config().general.status_cursor_on_changes = true;
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("<tab>"));

    ctx.update(&mut app, keys("<backtab>"));
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs"), "{buffer}");
    assert!(buffer.contains("b.rs"), "{buffer}");
    assert!(!buffer.contains("alpha changed"), "{buffer}");

    ctx.update(&mut app, keys("<backtab>"));
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha changed"), "{buffer}");
    assert!(buffer.contains("beta changed"), "{buffer}");
}

#[test]
fn files_headed_by_stat_rows_have_no_summary_row() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    ctx.config().general.file_header = crate::config::FileHeader::Stat;
    let _app = ctx.init_app();

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs | 2 +-"), "{buffer}");
    assert!(!buffer.contains("1 file changed"), "{buffer}");
}

/// Status is drawn once the renderer has drawn its first screen of the
/// unstaged diff, and the rest, staged files included, once it has drawn that.
#[test]
fn status_is_drawn_before_the_renderer_finishes() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str| {
        (1..=60)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "a.rs", &lines("old a"));
    commit(&ctx.dir, "b.rs", "old b\n");
    commit(&ctx.dir, "c.rs", "old c\n");
    fs::write(ctx.dir.join("a.rs"), lines("new a")).unwrap();
    fs::write(ctx.dir.join("b.rs"), "new b\n").unwrap();
    fs::write(ctx.dir.join("c.rs"), "new c\n").unwrap();
    run(&ctx.dir, &["git", "add", "c.rs"]);
    ctx.config().general.diff_renderer.command = super::pager::pausing_renderer();

    let started = Instant::now();
    let mut app = ctx.init_app_streaming_with_args(Default::default());

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(draws_lines_of(&app, "a.rs"));
    assert!(
        !draws_lines_of(&app, "b.rs"),
        "drawn before the renderer reached it"
    );

    while !(draws_lines_of(&app, "b.rs") && draws_lines_of(&app, "c.rs")) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the rest never arrived"
        );
        std::thread::sleep(Duration::from_millis(50));
        app.update(&mut ctx.term, None).unwrap();
    }
}

/// With files headed by stat rows, a folded file needs nothing from the
/// renderer, so status's first screen does not wait for the renderer to reach
/// its files.
#[test]
fn status_with_stat_rows_is_drawn_before_the_renderer_reaches_its_files() {
    let mut ctx = setup_clone!();
    super::pager::changed_files_and_a_renderer_stopping_after_two(&mut ctx, 12);
    ctx.config().general.file_header = crate::config::FileHeader::Stat;

    let started = Instant::now();
    ctx.init_app_streaming_with_args(Default::default());

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("f12.rs"), "{buffer}");
}

/// The branch is labelled as a field is, not in a sentence.
#[test]
fn the_branch_is_labelled_branch() {
    let mut ctx = setup_clone!();
    ctx.init_app();
    let buffer = ctx.redact_buffer();
    assert!(
        buffer.lines().next().unwrap().contains("Branch: main "),
        "{buffer}"
    );
}

/// The status view's diffs are drawn by the renderer too; one that cannot draw
/// them says why.
#[test]
fn a_renderer_that_fails_to_draw_status_says_why() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    ctx.config().general.diff_renderer.command = [
        "sh",
        "-c",
        "input=$(cat); case \"$input\" in *'diff --git'*) \
         echo 'Invalid color or style attribute: line' >&2; exit 2;; esac; \
         printf '%s\\n' \"$input\"",
    ]
    .map(String::from)
    .to_vec();

    let mut app = ctx.init_app();
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();
    ctx.update(&mut app, keys("gr"));

    let buffer = ctx.redact_buffer();
    assert!(
        buffer.contains("Invalid color or style attribute: line"),
        "{buffer}"
    );
}

/// Status started with a renderer that fails still shows what it can: the
/// branch, the changed files, the commits. It says why their diffs are not
/// drawn.
#[test]
fn status_with_a_renderer_that_fails_still_starts() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();
    ctx.config().general.diff_renderer.command = [
        "sh",
        "-c",
        "[ -z \"$(cat)\" ] || { echo 'Invalid color or style attribute: line' >&2; exit 2; }",
    ]
    .map(String::from)
    .to_vec();

    ctx.init_app();

    let buffer = ctx.redact_buffer();
    assert!(
        buffer.contains("Invalid color or style attribute: line"),
        "{buffer}"
    );
    assert!(buffer.contains("firstfile"), "{buffer}");
    assert!(buffer.contains("add firstfile"), "{buffer}");
}
