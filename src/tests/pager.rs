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

fn offers(app: &App, op: Op) -> bool {
    let item = app.state.screens.last().unwrap().get_selected_item();
    assert!(
        matches!(item.data, crate::item_data::ItemData::Hunk { .. }),
        "expected a hunk to be selected, got {:?}",
        item.data
    );
    op.implementation().get_action(&item.data).is_some()
}
