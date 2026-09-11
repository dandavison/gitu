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

fn twenty_lines(tenth: &str) -> String {
    (1..=20)
        .map(|i| {
            if i == 10 {
                format!("{tenth}\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect()
}

/// A diff gitu can ask git for again can be asked for differently: the context
/// around a change is a property of the question, not of the answer.
#[test]
fn widening_the_context_asks_git_for_more_of_the_file() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);
    assert!(
        !ctx.redact_buffer().contains("line 2 "),
        "three lines of context already reach line 2"
    );

    ctx.update(&mut app, keys("U8<enter>"));

    assert!(
        ctx.redact_buffer().contains("line 2 "),
        "the diff was not re-asked for:\n{}",
        ctx.redact_buffer()
    );
}

/// A commit's patch says which commit it is, so gitu can ask git for it again
/// — and ask differently. This is the case that matters for reading a commit
/// someone else wrote.
#[test]
fn widening_the_context_of_a_piped_commit() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    commit(&ctx.dir, "firstfile", &twenty_lines("changed"));

    let patch = run(&ctx.dir, &["git", "show", "HEAD"]);
    let mut app = ctx.init_app_with_patch(patch);
    assert!(!ctx.redact_buffer().contains("line 2 "));

    ctx.update(&mut app, keys("U8<enter>"));

    assert!(
        ctx.redact_buffer().contains("line 2 "),
        "the commit was not re-asked for:\n{}",
        ctx.redact_buffer()
    );
}

/// The key already said what is being set, so the prompt says nothing at all:
/// a line to type on, and no words explaining it every time.
#[test]
fn the_context_prompt_says_nothing() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);

    ctx.update(&mut app, keys("U"));

    let buffer = ctx.redact_buffer();
    assert!(!buffer.contains('\u{203a}'), "{buffer}");
    assert!(!buffer.contains("Context"), "{buffer}");
    assert!(!buffer.contains("default"), "{buffer}");
}

/// The whole function is the other thing worth asking for, and is a letter
/// rather than a number.
#[test]
fn the_whole_function_is_asked_for_by_letter() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);

    ctx.update(&mut app, keys("UW<enter>"));

    assert_eq!(app.state.context.as_deref(), Some("-W"));
}

/// Folding everything leaves one folded thing, not a stack of them: opening a
/// file shows the diff inside it, rather than another thing to open.
#[test]
fn opening_a_folded_file_shows_its_diff() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let patch = run(&ctx.dir, &["git", "diff"]);
    let mut app = ctx.init_app_with_patch(patch);

    ctx.update(&mut app, keys("<backtab><tab>"));

    assert!(
        ctx.redact_buffer().contains("changed"),
        "the file opened onto something still folded:\n{}",
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

/// A commit whose message is taller than the screen leaves a run of rows with
/// nothing to select on it that is longer than a page. Paging into that run
/// must not be pulled back onto the commit above it: holding space reaches the
/// end of the stream.
#[test]
fn paging_past_a_commit_taller_than_the_screen_reaches_the_end() {
    let mut ctx = setup_clone!();
    let body = (1..=30).fold(String::new(), |mut acc, i| {
        use std::fmt::Write as _;

        writeln!(acc, "body line {i}").unwrap();
        acc
    });
    fs::write(ctx.dir.join("tallfile"), "testing\n").unwrap();
    run(&ctx.dir, &["git", "add", "tallfile"]);
    run(
        &ctx.dir,
        &["git", "commit", "-m", &format!("a tall commit\n\n{body}")],
    );

    ctx.config().general.diff_colorizer.enabled = true;
    ctx.config().general.diff_colorizer.command = [
        "awk",
        r#"/^commit /{printf "\033]1717;1;C;;;%s\033\\", $2} {print}"#,
    ]
    .map(String::from)
    .to_vec();

    let mut app = ctx.init_app_with_patch(run(&ctx.dir, &["git", "log"]));
    ctx.update(&mut app, keys(&"<space>".repeat(10)));

    let buffer = ctx.redact_buffer();
    assert!(
        buffer.contains("add initial-file"),
        "paging stalled before the end:\n{buffer}"
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
