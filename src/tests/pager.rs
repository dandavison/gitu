//! Which ops a piped patch admits.
//!
//! There is no such thing as staging a line of `git show HEAD`: that patch is a
//! committed change, and applying part of it to the index is cherry-picking.
//! git tells its pager nothing about the command that produced the patch, so
//! its provenance has to be recognised rather than assumed.

use super::*;
use crate::{app::App, ops::Op};

#[test]
fn a_working_tree_patch_admits_stage_and_not_unstage() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    // git colours what it writes to its pager; provenance survives that.
    let patch = run(&ctx.dir, &["git", "-c", "color.diff=always", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("j"));

    assert!(offers(&app, Op::Stage));
    assert!(!offers(&app, Op::Unstage));
}

#[test]
fn an_index_patch_admits_unstage_and_not_stage() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    run(&ctx.dir, &["git", "add", "firstfile"]);

    let patch = run(&ctx.dir, &["git", "diff", "--cached"]);
    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("j"));

    assert!(offers(&app, Op::Unstage));
    assert!(!offers(&app, Op::Stage));
}

#[test]
fn a_commit_patch_admits_neither() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");

    let patch = run(&ctx.dir, &["git", "show", "HEAD"]);
    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("j"));

    assert!(!offers(&app, Op::Stage));
    assert!(!offers(&app, Op::Unstage));
}

#[test]
fn staging_a_hunk_of_a_piped_working_tree_patch_stages_it() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("js"));

    assert!(run(&ctx.dir, &["git", "diff", "--cached"]).contains("+changed"));
}

/// A patch recognised as the working tree is the working tree, so staging from
/// it leaves the view showing what is still unstaged — as the status screen
/// does. A view that keeps offering a change it has already staged is lying.
#[test]
fn a_staged_hunk_leaves_the_view_it_was_staged_from() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("js"));

    assert!(
        !ctx.redact_buffer().contains("changed"),
        "the staged line is still on screen:\n{}",
        ctx.redact_buffer()
    );
}

/// A commit's patch is not something git can be asked for again, so it stays
/// exactly as it arrived.
#[test]
fn a_commit_patch_stays_as_it_arrived() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");

    let patch = run(&ctx.dir, &["git", "show", "HEAD"]);
    let mut app = ctx.init_app_with_patch(patch);

    ctx.update(&mut app, keys("g"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("add firstfile"), "{buffer}");
    assert!(buffer.contains("+testing"), "{buffer}");
}

/// A log rendered by something that states where each commit begins is a log,
/// not a wall of text: its commits are targets, and the ops that act on a
/// commit apply. git tells its pager nothing, so the renderer has to.
#[test]
fn a_piped_log_with_commit_records_is_navigable() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let oid = run(&ctx.dir, &["git", "rev-parse", "HEAD"])
        .trim()
        .to_string();

    let app = ctx.init_app_with_patch(format!(
        "\x1b]1717;1\x1b\\\x1b]1717;1;C;;;{oid}\x1b\\commit {oid}\n\
         Author: Author Name <author@email.com>\n\
         \n\
         \x20   add firstfile\n"
    ));

    let item = app.state.screens.last().unwrap().get_selected_item();
    assert!(
        matches!(&item.data, crate::item_data::ItemData::Commit { oid: o, .. } if *o == oid),
        "got {:?}",
        item.data
    );
    assert!(Op::Show.implementation().get_action(&item.data).is_some());
    assert!(
        Op::CopyHash
            .implementation()
            .get_action(&item.data)
            .is_some()
    );
}

/// A renderer names the commit as git printed it, and most log formats print
/// `%h`. An id gitu cannot resolve costs more than the commit ops: the rows
/// fall back to being decoration, and the cursor cannot move at all.
#[test]
fn a_log_naming_its_commits_by_abbreviation_is_navigable() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let short = run(&ctx.dir, &["git", "rev-parse", "--short", "HEAD"])
        .trim()
        .to_string();
    let oid = run(&ctx.dir, &["git", "rev-parse", "HEAD"])
        .trim()
        .to_string();

    let app = ctx.init_app_with_patch(format!(
        "\x1b]1717;1;C;;;{short}\x1b\\▸ {short} Author Name\n\
         \n\
         \x20   add firstfile\n"
    ));

    let item = app.state.screens.last().unwrap().get_selected_item();
    assert!(
        matches!(&item.data, crate::item_data::ItemData::Commit { oid: o, .. } if *o == oid),
        "got {:?}",
        item.data
    );
}

/// A view is legitimately empty: `git diff --cached` with nothing staged is no
/// bytes at all. A screen with no rows must still answer what is selected, or
/// the next keypress indexes into nothing.
#[test]
fn a_key_press_on_an_empty_view_does_nothing() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app_with_patch(String::new());

    ctx.update(&mut app, keys("j"));
}

/// The same view, reached the other way: everything it showed has been staged.
#[test]
fn a_key_press_after_staging_everything_does_nothing() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);

    ctx.update(&mut app, keys("jsj"));
}

/// Output gitu can find no structure in is still the renderer's to draw. Shown
/// as it arrived it would carry git's colours, and setting gitu as the pager
/// would cost the rendering of everything that is not a diff.
#[test]
fn output_with_no_structure_is_still_rendered() {
    let mut ctx = setup_clone!();
    ctx.config().general.diff_colorizer.enabled = true;
    ctx.config().general.diff_colorizer.command =
        ["sed", "s/^/rendered /"].map(String::from).to_vec();

    ctx.init_app_with_patch("plain output\n".to_string());

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("rendered plain output"), "{buffer}");
}

/// And rendering it is what makes a log a log: git hands its pager an unmarked
/// wall of text, and the renderer is what says where each commit begins. So
/// gitu needs no pipeline in front of it, only its own renderer.
#[test]
fn a_raw_log_becomes_a_log_view_when_the_renderer_marks_commits() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let oid = run(&ctx.dir, &["git", "rev-parse", "HEAD"])
        .trim()
        .to_string();

    ctx.config().general.diff_colorizer.enabled = true;
    ctx.config().general.diff_colorizer.command = [
        "awk",
        r#"/^commit /{printf "\033]1717;1;C;;;%s\033\\", $2} {print}"#,
    ]
    .map(String::from)
    .to_vec();

    let app = ctx.init_app_with_patch(run(&ctx.dir, &["git", "log"]));

    let item = app.state.screens.last().unwrap().get_selected_item();
    assert!(
        matches!(&item.data, crate::item_data::ItemData::Commit { oid: o, .. } if *o == oid),
        "got {:?}",
        item.data
    );
}

fn offers(app: &App, op: Op) -> bool {
    let item = app.state.screens.last().unwrap().get_selected_item();
    assert!(
        matches!(item.data, crate::item_data::ItemData::Hunk { .. }),
        "expected a hunk to be selected, got {:?}",
        item.data
    );
    op.implementation().get_action(&item.data).is_some()
}
