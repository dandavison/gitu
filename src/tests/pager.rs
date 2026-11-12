//! Which ops a piped patch admits.
//!
//! There is no such thing as staging a line of `git show HEAD`: that patch is a
//! committed change, and applying part of it to the index is cherry-picking.
//! git tells its pager nothing about the command that produced the patch, so
//! gitu finds it in the process tree; what it finds is what the patch is.

use super::*;
use crate::{app::App, ops::Op};
use std::time::{Duration, Instant};

#[test]
fn a_working_tree_patch_admits_stage_and_not_unstage() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    // git colours what it writes to its pager; the command survives that.
    let mut app = ctx.init_app_as_pager_of(&["git", "-c", "color.diff=always", "diff"]);
    ctx.update(&mut app, keys("<ctrl+j>"));

    assert!(offers(&app, Op::Stage));
    assert!(!offers(&app, Op::Unstage));
}

#[test]
fn an_index_patch_admits_unstage_and_not_stage() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    run(&ctx.dir, &["git", "add", "firstfile"]);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff", "--cached"]);
    ctx.update(&mut app, keys("<ctrl+j>"));

    assert!(offers(&app, Op::Unstage));
    assert!(!offers(&app, Op::Stage));
}

#[test]
fn a_commit_patch_admits_neither() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");

    let mut app = ctx.init_app_as_pager_of(&["git", "show", "HEAD"]);
    ctx.update(&mut app, keys("<ctrl+j>"));

    assert!(!offers(&app, Op::Stage));
    assert!(!offers(&app, Op::Unstage));
}

#[test]
fn staging_a_hunk_of_a_piped_working_tree_patch_stages_it() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("<ctrl+j>s"));

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

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("<ctrl+j>s"));

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

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
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

    let mut app = ctx.init_app_as_pager_of(&["git", "show", "HEAD"]);
    assert!(!ctx.redact_buffer().contains("line 2 "));

    ctx.update(&mut app, keys("U8<enter>"));

    assert!(
        ctx.redact_buffer().contains("line 2 "),
        "the commit was not re-asked for:\n{}",
        ctx.redact_buffer()
    );
}

/// Every number asked for is passed to git, 3 included: git's own default for
/// the context may be configured to something else.
#[test]
fn asking_for_three_lines_overrides_configured_context() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();
    run(&ctx.dir, &["git", "config", "diff.context", "8"]);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    assert!(ctx.redact_buffer().contains("line 2 "));

    ctx.update(&mut app, keys("U3<enter>"));

    assert!(
        !ctx.redact_buffer().contains("line 2 "),
        "{}",
        ctx.redact_buffer()
    );
}

/// The context asked for is offered for editing, and asking for none goes back
/// to the command as it was written.
#[test]
fn an_empty_context_answer_asks_for_no_context_flag() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("U8<enter>U"));
    assert_eq!(ctx.prompt_line(), "8|");

    ctx.update(&mut app, keys("U<ctrl+u><enter>"));
    assert_eq!(app.state.context, None);
}

/// The key already said what is being set, so the prompt says nothing at all:
/// a line to type on, and no words explaining it every time.
#[test]
fn the_context_prompt_says_nothing() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

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

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("UW<enter>"));

    assert_eq!(app.state.context.as_deref(), Some("-W"));
}

/// Every property of a view is in the command it is the output of, so the
/// command itself is what is offered for editing — prefilled, and with nothing
/// said about it.
#[test]
fn the_command_is_offered_for_editing_as_it_is_being_asked() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "-c", "color.diff=always", "diff"]);
    ctx.update(&mut app, keys("U8<enter>:"));

    let buffer = ctx.redact_buffer();
    assert!(
        buffer.contains("git -c color.diff=always diff -U8"),
        "{buffer}"
    );
}

/// The edit is a different question, so the answer is a different view: what
/// the patch is a diff of has changed, and with it which ops mean anything.
#[test]
fn editing_the_command_asks_the_new_question() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    run(&ctx.dir, &["git", "add", "firstfile"]);
    fs::write(ctx.dir.join("firstfile"), "changed\nagain\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("<ctrl+j>"));
    assert!(offers(&app, Op::Stage));
    assert!(ctx.redact_buffer().contains("+again"));

    ctx.update(&mut app, keys(": --cached<enter><ctrl+j>"));

    assert!(offers(&app, Op::Unstage));
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("+changed"), "{buffer}");
    assert!(!buffer.contains("+again"), "{buffer}");
}

/// The line handed over says what context it is asking for, so accepting it
/// keeps that: the command is the whole of what the view is, and the session's
/// own context override is spent.
#[test]
fn an_edit_takes_over_the_context_asked_for() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("U8<enter>:<enter>"));

    assert_eq!(app.state.context, None);
    assert!(
        ctx.redact_buffer().contains("line 2 "),
        "the widened context was lost:\n{}",
        ctx.redact_buffer()
    );
}

/// ... and in a diffstat the context is for the patches its files unfold to: the
/// view stays a diffstat.
#[test]
fn an_edit_of_a_stat_keeps_it_a_stat_with_the_context_asked_for() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", &twenty_lines("line 10"));
    fs::write(ctx.dir.join("firstfile"), twenty_lines("changed")).unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff", "--stat"]);
    ctx.update(&mut app, keys("U8<enter>:<enter>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("firstfile | 2 +-"), "{buffer}");
    assert!(!buffer.contains("modified   firstfile"), "{buffer}");

    ctx.update(&mut app, keys("<tab>"));
    assert!(
        ctx.redact_buffer().contains("line 2 "),
        "the widened context was lost:\n{}",
        ctx.redact_buffer()
    );
}

/// A command the user can edit is one the user can get wrong. git says what it
/// thought of it, and the view it could not answer stays as it was.
#[test]
fn a_command_git_refuses_says_so_and_changes_nothing() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys(": --nonsense<enter>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("invalid option: --nonsense"), "{buffer}");
    assert!(buffer.contains("+changed"), "{buffer}");
}

/// The editing keys reach the line being edited, rather than the view behind
/// it: `ctrl+w` in a prompt takes back a word, and does not scroll.
#[test]
fn a_command_can_be_edited_with_the_readline_keys() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff", "--cached"]);
    ctx.update(&mut app, keys(":<ctrl+w>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("git diff "), "{buffer}");
    assert!(!buffer.contains("--cached"), "{buffer}");
}

/// The cursor is drawn where the next character will land. A line that shows
/// it at the end while typing inserts in the middle is unusable.
#[test]
fn the_cursor_is_drawn_at_the_insertion_point() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    // A prompt left open by one batch of keys is abandoned before the next, so
    // each of these is a fresh prompt typed into from the start.
    ctx.update(&mut app, keys(":"));
    assert_eq!(ctx.prompt_line(), "git diff|");

    ctx.update(&mut app, keys(":<ctrl+a>"));
    assert_eq!(ctx.prompt_line(), "|git diff");

    ctx.update(&mut app, keys(":<ctrl+a><alt+f>"));
    assert_eq!(ctx.prompt_line(), "git| diff");

    ctx.update(&mut app, keys(":<ctrl+a><alt+f>X"));
    assert_eq!(ctx.prompt_line(), "gitX| diff");
}

/// A patch with no command behind it has no question to edit.
#[test]
fn a_patch_with_no_command_cannot_be_edited() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");

    let patch = run(&ctx.dir, &["git", "show", "HEAD"]);
    let mut app = ctx.init_app_with_patch(patch);

    ctx.update(&mut app, keys(":"));

    assert!(
        ctx.redact_buffer()
            .contains("not something gitu asked git for"),
        "{}",
        ctx.redact_buffer()
    );
}

/// Two files whose contents say which is which, so that a view can be asserted
/// to have dropped one of them.
fn two_changed_files(ctx: &TestContext) {
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "a_test.rs", "beta\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    fs::write(ctx.dir.join("a_test.rs"), "beta changed\n").unwrap();
}

/// A third file whose every line changed, so the patch is taller than the
/// test terminal's screen.
fn taller_than_the_screen(ctx: &TestContext) {
    let lines = |suffix: &str| {
        (1..=30)
            .map(|i| format!("{i}{suffix}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "tall.rs", &lines(""));
    fs::write(ctx.dir.join("tall.rs"), lines(" changed")).unwrap();
}

/// A hidden file is not "diff not displayed": it is absent, as if the patch
/// never had it. What is left is a diff like any other.
#[test]
fn hiding_the_file_under_the_cursor_drops_it_from_the_view() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("kdj"));

    let buffer = ctx.redact_buffer();
    assert!(!buffer.contains("alpha"), "{buffer}");
    assert!(buffer.contains("beta"), "{buffer}");
    assert!(offers(&app, Op::Stage));
}

/// Like `d` in the rebase list, `d` acts on the row under the cursor: within a
/// file's hunks there is no file there to drop.
#[test]
fn hiding_a_file_needs_the_cursor_on_its_header() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("d"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha"), "{buffer}");
    assert!(buffer.contains("beta"), "{buffer}");
}

/// The paths are the user's to say, in git's own language less the magic: a
/// bare pattern limits the view, `!` drops what it matches.
#[test]
fn the_files_to_show_can_be_said_by_pattern() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("&!*_test.rs<enter>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha"), "{buffer}");
    assert!(!buffer.contains("beta"), "{buffer}");
}

/// Hiding a file is an edit of the command like any other, so it is there to
/// be read back, undone, or added to.
#[test]
fn the_files_asked_for_come_back_for_editing() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("kd&"));
    assert!(
        ctx.redact_buffer().contains("Files: › !a.rs"),
        "{}",
        ctx.redact_buffer()
    );

    ctx.update(&mut app, keys("<enter>:"));
    assert!(
        ctx.redact_buffer()
            .contains("git diff -- ':(top,exclude)a.rs'"),
        "{}",
        ctx.redact_buffer()
    );
}

/// A command reached without typing it, by hiding a file, is in the command
/// history as one typed at `:` is.
#[test]
fn hiding_a_file_puts_the_command_in_the_history() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("kd"));
    drop(app);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys(":<up>"));
    assert_eq!(ctx.prompt_line(), "git diff -- ':(top,exclude)a.rs'|");
}

/// Commands and file patterns are two different kinds of question, and what
/// was answered before remains available after gitu itself has exited.
#[test]
fn command_and_file_histories_persist_separately() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(
        &mut app,
        keys(
            "&!a_test.rs<enter>&<ctrl+u>!a.rs<enter>\
             : HEAD<enter>:<ctrl+u>git diff --cached<enter>",
        ),
    );
    drop(app);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys(":<up>"));
    assert_eq!(ctx.prompt_line(), "git diff --cached|");

    ctx.update(&mut app, keys("&<up>"));
    assert_eq!(ctx.prompt_line(), "? Files: › !a.rs|");

    ctx.update(&mut app, keys(":<up><up><down>"));
    assert_eq!(ctx.prompt_line(), "git diff --cached|");
}

/// Git shares one object store between linked worktrees, but prompt history is
/// about the question asked in one checkout and must not leak into another.
#[test]
fn command_history_is_scoped_to_the_worktree() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys(": HEAD<enter>"));
    drop(app);

    // Relative, since git cannot create the directories of a Windows `\\?\` path.
    run(
        &ctx.dir,
        &["git", "worktree", "add", "-b", "other", "../other-worktree"],
    );
    let other = ctx.dir.parent().unwrap().join("other-worktree");
    let mut app = ctx.init_app_as_pager_of_at(other, &["git", "diff"]);
    ctx.update(&mut app, keys(":<up>"));

    assert_eq!(ctx.prompt_line(), "git diff|");
}

/// The view shows the command's output only: what is being asked is for `:`
/// to say, edited or not.
#[test]
fn an_edited_command_is_not_shown_in_the_view() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("kd"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("beta"), "{buffer}");
    assert!(!buffer.contains("git diff"), "{buffer}");
}

/// Folding everything leaves one folded thing, not a stack of them: opening a
/// file shows the diff inside it, rather than another thing to open.
#[test]
fn opening_a_folded_file_shows_its_diff() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("<backtab><tab>"));

    assert!(
        ctx.redact_buffer().contains("changed"),
        "the file opened onto something still folded:\n{}",
        ctx.redact_buffer()
    );
}

/// A patch gitu found no command for — a saved file, a process already gone —
/// is not something git can be asked for again, so it stays exactly as it
/// arrived.
#[test]
fn a_patch_with_no_command_stays_as_it_arrived() {
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

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("jsj"));
}

/// Output gitu can find no structure in is still the renderer's to draw. Shown
/// as it arrived it would carry git's colours, and setting gitu as the pager
/// would cost the rendering of everything that is not a diff.
#[test]
fn output_with_no_structure_is_still_rendered() {
    let mut ctx = setup_clone!();
    ctx.config().general.diff_renderer.command =
        ["sed", "s/^/rendered /"].map(String::from).to_vec();

    ctx.init_app_with_patch("plain output\n".to_string());

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("rendered plain output"), "{buffer}");
}

/// `/` finds rendered text, and `n`/`N` repeat in either direction. Repeating
/// past an end wraps, as it does in a pager.
#[test]
fn rendered_text_can_be_searched() {
    let mut ctx = setup_clone!();
    let mut app = ctx.init_app_with_patch("start\nmatch one\nmiddle\nmatch two\n".to_owned());

    ctx.update(&mut app, keys("/match<enter>"));
    assert_eq!(selected_rendered_row(&app), "match one");

    ctx.update(&mut app, keys("n"));
    assert_eq!(selected_rendered_row(&app), "match two");

    ctx.update(&mut app, keys("n"));
    assert_eq!(selected_rendered_row(&app), "match one");

    ctx.update(&mut app, keys("N"));
    assert_eq!(selected_rendered_row(&app), "match two");
}

/// Search patterns are regular expressions over the renderer's visible text,
/// not over its ANSI control sequences.
#[test]
fn search_is_a_regex_over_rendered_text() {
    let mut ctx = setup_clone!();
    let mut app = ctx
        .init_app_with_patch("\x1b[31mfirst needle\x1b[0m\nsecond needle\nnot this\n".to_owned());

    ctx.update(&mut app, keys(r"/^second .*le$<enter>"));

    assert_eq!(selected_rendered_row(&app), "second needle");
}

/// Folding changes presentation, not what the buffer contains: searching for
/// a hidden row opens its containing sections and lands on it.
#[test]
fn search_reveals_a_match_inside_a_fold() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "before\n");
    fs::write(ctx.dir.join("firstfile"), "changed\n").unwrap();
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("<backtab>/changed<enter>"));

    assert!(
        matches!(
            app.screen().get_selected_item().data,
            crate::item_data::ItemData::HunkLine { .. }
        ),
        "search did not land on the changed line"
    );
    assert!(ctx.redact_buffer().contains("+changed"));
}

fn selected_rendered_row(app: &App) -> String {
    app.screen()
        .get_selected_item()
        .rendered
        .as_ref()
        .map(|row| {
            row.iter()
                .map(|(text, _)| crate::ui::display_text(text))
                .collect()
        })
        .unwrap_or_default()
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

    ctx.config().general.diff_renderer.command = [
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

    ctx.config().general.diff_renderer.command = [
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

/// A context line carries no change, so with `visit_context_lines` off the
/// cursor passes over it, onto what a line-level op can act on.
#[test]
fn context_lines_are_skipped_when_configured() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "one\ntwo\nthree\n");
    fs::write(ctx.dir.join("firstfile"), "one\nCHANGED\nthree\n").unwrap();
    ctx.config().general.visit_context_lines = false;

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("j"));

    assert_eq!(selected_hunk_line(&app), Some(1));
}

/// Reading a patch is line-by-line, and skipping the unchanged lines makes the
/// cursor jump, so by default it stops on all of them.
#[test]
fn context_lines_are_visited_by_default() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "one\ntwo\nthree\n");
    fs::write(ctx.dir.join("firstfile"), "one\nCHANGED\nthree\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    for line_i in 0..4 {
        ctx.update(&mut app, keys("j"));
        assert_eq!(selected_hunk_line(&app), Some(line_i));
    }
}

/// And the renderer's rows are the same lines, so the cursor stops on the same
/// ones.
#[test]
fn rendered_context_lines_are_visited_when_configured() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "one\ntwo\nthree\n");
    fs::write(ctx.dir.join("firstfile"), "one\nCHANGED\nthree\n").unwrap();
    ctx.config().general.visit_context_lines = true;

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    for line_i in 0..4 {
        ctx.update(&mut app, keys("j"));
        assert_eq!(selected_hunk_line(&app), Some(line_i));
    }
}

/// A renderer that draws as delta does with its styles plain: for the same
/// input, `delta` with the arguments `GITU_TEST_RENDERER=delta` gives it (in
/// `helpers/ui.rs`) writes the same bytes. It speaks the protocol, so its rows
/// carry the metadata that makes them content lines rather than plain text, and
/// it marks each commit as delta does for rows matching
/// `commit-regex = ^(▸|commit )`. It draws a file's header as delta does, in
/// place of git's header rows, and keeps git's colours only off a hunk's lines.
/// Only a hunk's lines are content: a commit message's are indented as context
/// lines are.
pub(super) fn annotating_renderer() -> Vec<String> {
    // The program has no `\\`: on Windows, awk (MSYS2's) reads `\\` in its
    // command-line argument as a single backslash, so a backslash is `bs`.
    [
        "awk",
        r#"
        BEGIN{bs=sprintf("%c",92);osc="\033]1717;1";st="\033" bs;handshake=osc st}
        {raw=$0;gsub(/\033\[[0-9;]*m/,"");printf "%s",handshake;handshake=""}
        !/^[-+ ]/&&substr($0,1,1)!=bs{hunk=0}
        header&&/^(@@|diff |(commit|▸) [0-9a-f]+)/{file_header()}
        /^(commit|▸) [0-9a-f]+/{printf "%s;C;;;%s%s%s\n",osc,$2,st,raw;next}
        /^diff --git /{file=$0;sub(/^diff --git a\/.* b\//,"",file);label=file;suffix="";header=1;next}
        /^diff --cc /{file=substr($0,11);label=file;suffix="";header=1;next}
        /^new file mode /{label="added: " file}
        /^deleted file mode /{label="removed: " file}
        /^rename from /{from=substr($0,13)}
        /^rename to /{label="renamed: " from " ⟶   " file}
        /^copy from /{from=substr($0,11)}
        /^copy to /{label="copied: " from " ⟶   " file}
        /^old mode /{oldmode=$3}
        /^new mode /{suffix=oldmode=="100644"&&$3=="100755"?" (mode +x)":oldmode=="100755"&&$3=="100644"?" (mode -x)":" (mode " oldmode " ⟶   " $3 ")"}
        /^Binary files /{suffix=" (binary file)"}
        header{next}
        /^@@ /{o=$2;n=$3;sub(/^-/,"",o);sub(/^\+/,"",n);split(o,a,",");split(n,b,",");old=a[1];new=b[1];hunk=1;printf "%s;h;%d;;%s%s%s\n",osc,new,file,st,raw;next}
        hunk&&/^\+/{printf "%s;a;%d;;%s%s%s\n",osc,new++,file,st,$0;next}
        hunk&&/^-/{printf "%s;d;%d;%d;%s%s%s\n",osc,new,old++,file,st,$0;next}
        hunk&&/^ /{printf "%s;c;%d;;%s%s%s\n",osc,new++,file,st,$0;old++;next}
        {print raw}
        END{if(header)file_header();printf "%s",handshake}
        function file_header(){printf "\n%s;f;;;%s%s%s%s\n",osc,file,st,label,suffix;header=0}
        "#,
    ]
    .map(String::from)
    .to_vec()
}

fn selected_hunk_line(app: &App) -> Option<usize> {
    match app.screen().get_selected_item().data {
        crate::item_data::ItemData::HunkLine { line_i, .. } => Some(line_i),
        _ => None,
    }
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

/// A re-render can lay the same patch out in fewer rows: side-by-side fuses a
/// changed pair into one. The viewport has to come back up to the content it is
/// now taller than, rather than stay where it was with blank rows below it.
#[test]
fn a_shorter_rendering_still_fills_the_screen() {
    let mut ctx = setup_clone!();
    ctx.config().general.diff_renderer.features = vec!["side-by-side".into()];
    ctx.config().general.diff_renderer.command = fusing_renderer();

    // Twice the rows of the 20-row test terminal, halved by the feature: the
    // screen can be filled either way.
    let patch = (1..=100).map(|i| format!("line {i}\n")).collect::<String>();
    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("G"));
    // The two rows gitu keeps below the end of the content, and no more.
    assert!(
        blank_rows_at_the_bottom(&ctx.redact_buffer()) <= 2,
        "before the re-render"
    );

    ctx.update(&mut app, keys("H<enter>"));
    let buffer = ctx.redact_buffer();
    assert!(blank_rows_at_the_bottom(&buffer) <= 2, "{buffer}");
}

/// A re-render, such as turning on line numbers, leaves the cursor's line on
/// the row of the screen it was on, though the first screenfuls of the new
/// rendering end not far below it.
#[test]
fn a_rerender_keeps_the_cursor_on_its_screen_row() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str| {
        (1..=60)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "a.rs", &lines("old"));
    fs::write(ctx.dir.join("a.rs"), lines("new")).unwrap();
    let renderer = &mut ctx.config().general.diff_renderer;
    renderer.features = vec!["line-numbers".into()];
    renderer.feature_keys.insert("line-numbers".into(), 'L');
    renderer.command = pausing_renderer();

    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);
    finish_drawing(&mut ctx, &mut app);
    ctx.update(
        &mut app,
        keys(&format!(
            "{}{}",
            "<ctrl+j>".repeat(90),
            "<ctrl+k>".repeat(15)
        )),
    );
    let before = (selected_hunk_line(&app), cursor_screen_row(&ctx));

    ctx.update(&mut app, keys("L"));
    assert_eq!(
        (selected_hunk_line(&app), cursor_screen_row(&ctx)),
        before,
        "{}",
        ctx.redact_buffer()
    );

    finish_drawing(&mut ctx, &mut app);
    ctx.update(&mut app, vec![]);
    assert_eq!(
        (selected_hunk_line(&app), cursor_screen_row(&ctx)),
        before,
        "{}",
        ctx.redact_buffer()
    );
}

fn cursor_screen_row(ctx: &TestContext) -> Option<usize> {
    ctx.redact_buffer()
        .lines()
        .position(|row| row.starts_with('\u{258c}'))
}

/// A renderer that halves its rows once a feature is on, as a side-by-side one
/// does to a run of changed lines.
fn fusing_renderer() -> Vec<String> {
    [
        "sh",
        "-c",
        r#"if [ -n "$DELTA_FEATURES" ]; then awk 'NR%2==1'; else cat; fi"#,
    ]
    .map(String::from)
    .to_vec()
}

/// A one-hunk patch of `file`, whose content lines say which file they are
/// from, so a view can be asserted to have dropped one of two files.
fn one_hunk_patch(file: &str) -> String {
    format!(
        "diff --git a/{file} b/{file}\n\
         index 1111111..2222222 100644\n\
         --- a/{file}\n\
         +++ b/{file}\n\
         @@ -1 +1 @@\n\
         -{file} was\n\
         +{file} is\n"
    )
}

/// gitu reads all of its input before it draws anything, so input it cannot get
/// to the end of quickly has to stop somewhere. A file diff cut short is dropped
/// whole: what is left is a patch, not a fragment of one.
#[test]
fn input_past_the_limit_is_cut_back_to_whole_file_diffs() {
    let mut ctx = setup_clone!();
    let first = one_hunk_patch("a.rs");
    let patch = format!("{first}{}", one_hunk_patch("b.rs"));
    ctx.config().general.max_input_bytes = (first.len() + 60) as u64;

    let mut app = ctx.init_app_with_patch(patch);
    ctx.update(&mut app, keys("j"));

    let buffer = ctx.redact_buffer();
    assert!(!buffer.contains("b.rs"), "{buffer}");
    // Where the reading stopped, which is not where anyone starts reading.
    assert!(
        buffer.find("a.rs is") < buffer.find("input truncated at"),
        "{buffer}"
    );
}

/// A hunk missing its last lines is one git will not apply, so the hunk the
/// limit fell inside goes too.
#[test]
fn a_hunk_the_limit_cut_short_is_dropped() {
    let mut ctx = setup_clone!();
    let patch = "diff --git a/f.rs b/f.rs\n\
                 index 1111111..2222222 100644\n\
                 --- a/f.rs\n\
                 +++ b/f.rs\n\
                 @@ -1 +1 @@\n\
                 -first was\n\
                 +first is\n\
                 @@ -9 +9 @@\n\
                 -second was\n\
                 +second is\n";
    ctx.config().general.max_input_bytes = (patch.find("+second").unwrap() + 4) as u64;

    let mut app = ctx.init_app_with_patch(patch.to_owned());
    ctx.update(&mut app, keys("j"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("first is"), "{buffer}");
    assert!(!buffer.contains("second"), "{buffer}");
}

/// git colours what it writes to its pager, and the line's first character is
/// what says whether a file diff or a hunk starts there.
#[test]
fn a_coloured_patch_is_cut_back_at_the_same_place() {
    let mut ctx = setup_clone!();
    let first = one_hunk_patch("a.rs");
    let coloured = |patch: String| {
        patch
            .lines()
            .map(|line| format!("\x1b[1m{line}\x1b[m\n"))
            .collect::<String>()
    };
    let patch = coloured(format!("{first}{}", one_hunk_patch("b.rs")));
    ctx.config().general.max_input_bytes = (coloured(first).len() + 60) as u64;

    let _app = ctx.init_app_with_patch(patch);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs is"), "{buffer}");
    assert!(!buffer.contains("b.rs"), "{buffer}");
}

/// A command gitu puts again can say as much as the pipe did, so what comes
/// back is read to the same limit.
#[test]
fn a_command_that_says_more_than_the_limit_is_cut_back_too() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    // Into the second file's diff, as git writes it to gitu: in colour.
    let coloured = run(&ctx.dir, &["git", "-c", "color.ui=always", "diff"]);
    let limit = coloured.find("a/a_test.rs").unwrap() + 20;
    ctx.config().general.max_input_bytes = limit as u64;

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("<ctrl+j>"));

    let buffer = ctx.redact_buffer();
    assert!(
        buffer.contains(&format!("input truncated at {limit} bytes")),
        "{buffer}"
    );
    assert!(buffer.contains("alpha"), "{buffer}");
    assert!(!buffer.contains("beta"), "{buffer}");
    assert!(offers(&app, Op::Stage), "what is left is still a patch");
}

/// Rows of the viewport with nothing drawn on them, counted up from the last.
/// [`TestContext::redact_buffer`] ends with a `styles_hash` line, which is not
/// one of them.
fn blank_rows_at_the_bottom(buffer: &str) -> usize {
    buffer
        .lines()
        .filter(|row| row.ends_with('|'))
        .rev()
        .take_while(|row| row.trim_end_matches(['|', ' ']).is_empty())
        .count()
}

/// `--quit-if-one-screen`, as less's: a patch that fits on the terminal is
/// printed onto it, so that it is still there once gitu has exited.
#[test]
fn a_patch_that_fits_on_one_screen_is_printed() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.term.clear().unwrap();
    ctx.term.queue_move_cursor(0, 0).unwrap();

    assert!(app.print_if_one_screen(&mut ctx.term).unwrap());
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn a_patch_longer_than_one_screen_is_not_printed() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str| {
        (1..=20)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "firstfile", &lines("old"));
    fs::write(ctx.dir.join("firstfile"), lines("new")).unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.term.clear().unwrap();
    ctx.term.queue_move_cursor(0, 0).unwrap();

    assert!(!app.print_if_one_screen(&mut ctx.term).unwrap());
    assert!(ctx.physical_screen().trim().is_empty());
}

/// The help menu's columns are headed by what is in them: commands, submenus,
/// and the commands that act on the row under the cursor, headed by the kind of
/// row it is. The row itself, as a renderer drew it, would be clipped to a
/// fragment in a column.
#[test]
fn the_help_menu_heads_its_columns_by_what_is_in_them() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = hunk_header_renderer();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.resize(&mut app, &[super::HELP_FITS]);
    ctx.update(&mut app, keys("?"));
    assert_eq!(menu_headings(&ctx), ["Commands", "Submenus", "Hunk"]);

    ctx.update(&mut app, keys("<esc>j?"));
    assert_eq!(menu_headings(&ctx), ["Commands", "Submenus", "Line"]);
}

/// The words on the first row of the menu, below its separator.
fn menu_headings(ctx: &TestContext) -> Vec<String> {
    let buffer = ctx.redact_buffer();
    let mut rows = buffer.lines().skip_while(|row| !row.starts_with('─'));
    let headings = rows.nth(1).unwrap_or_default();
    headings
        .trim_end_matches('|')
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// [`annotating_renderer`], drawing each hunk header its own way as delta
/// does, and saying so, so that the row gitu folds the hunk on is the
/// renderer's.
fn hunk_header_renderer() -> Vec<String> {
    annotating_renderer()
        .into_iter()
        .map(|arg| {
            arg.replace(
                r#"%s;h;%d;;%s%s%s\n",osc,new,file,st,raw;next}"#,
                r#"%s;h;%d;;%srendered hunk %d\n",osc,new,file,st,new;next}"#,
            )
        })
        .collect()
}

/// The file header the renderer draws (delta's `file-style` and
/// `file-decoration-style`) is the file's row: it folds the file, and the
/// file's commands act from it.
#[test]
fn the_renderers_file_header_folds_and_stages_its_file() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = file_header_renderer();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("~ firstfile ~"), "{buffer}");
    assert!(buffer.contains("==========="), "{buffer}");
    assert!(!buffer.contains("modified   firstfile"), "{buffer}");

    ctx.update(&mut app, keys("k<tab>"));
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("~ firstfile ~"), "{buffer}");
    assert!(!buffer.contains("==========="), "{buffer}");
    assert!(!buffer.contains("changed"), "{buffer}");

    ctx.update(&mut app, keys("s"));
    assert!(run(&ctx.dir, &["git", "diff", "--cached"]).contains("+changed"));
}

/// A diffstat asked for with the patch is the patch with each file headed by
/// its stat row, unfolded: one stat row per file, whatever heads files
/// otherwise.
#[test]
fn a_patch_asked_for_with_its_stat_has_one_stat_row_per_file() {
    for file_header in [
        crate::config::FileHeader::Delta,
        crate::config::FileHeader::Stat,
    ] {
        for argv in [
            ["git", "diff", "-p", "--stat"],
            ["git", "diff", "--patch-with-stat", "-U3"],
        ] {
            let mut ctx = setup_clone!();
            ctx.config().general.file_header = file_header;
            commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
            fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
            ctx.config().general.diff_renderer.command = file_header_renderer();

            ctx.init_app_as_pager_of(&argv);

            let buffer = ctx.redact_buffer();
            assert_eq!(buffer.matches("firstfile | 2 +-").count(), 1, "{buffer}");
            assert!(!buffer.contains("~ firstfile ~"), "{buffer}");
            assert!(buffer.contains("changed"), "{buffer}");
        }
    }
}

/// A file unfolded from a diffstat is headed by its stat row in place of the
/// renderer's file header.
#[test]
fn a_stat_file_unfolds_under_its_stat_row_not_the_renderers_header() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = file_header_renderer();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff", "--stat"]);
    ctx.update(&mut app, keys("<tab>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("firstfile | 2 +-"), "{buffer}");
    assert!(buffer.contains("changed"), "{buffer}");
    assert!(!buffer.contains("~ firstfile ~"), "{buffer}");
    assert!(!buffer.contains("==========="), "{buffer}");
}

/// [`annotating_renderer`], drawing each file's header its own way, named and
/// underlined, as delta does.
fn file_header_renderer() -> Vec<String> {
    annotating_renderer()
        .into_iter()
        .map(|arg| {
            arg.replace(
                r#"function file_header(){printf "\n%s;f;;;%s%s%s%s\n",osc,file,st,label,suffix;header=0}"#,
                r#"function file_header(){printf "%s;f;;;%s%s~ %s ~\n%s;f;;;%s%s===========\n",osc,file,st,file,osc,file,st;header=0}"#,
            )
        })
        .collect()
}

/// Input that says whether it was read.
struct Watched<'a> {
    text: &'a [u8],
    read: &'a std::cell::Cell<bool>,
}

impl std::io::Read for Watched<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read.set(true);
        self.text.read(buf)
    }
}

/// Once gitu has found the command git ran, it asks git again itself and
/// discards what git piped, so it does not wait for git to write it.
#[test]
fn what_git_piped_is_not_read_when_its_command_was_found() {
    let read = std::cell::Cell::new(false);
    let input = Watched {
        text: b"diff --git a/f b/f\n",
        read: &read,
    };

    crate::paged(input, Some(vec!["git".into(), "diff".into()]), None, 1_000).unwrap();

    assert!(!read.get());
}

#[test]
fn what_git_piped_is_read_when_its_command_was_not_found() {
    let read = std::cell::Cell::new(false);
    let input = Watched {
        text: b"diff --git a/f b/f\n",
        read: &read,
    };

    let paged = crate::paged(input, None, None, 1_000).unwrap();

    assert_eq!(paged.text, "diff --git a/f b/f\n");
}

/// As git's pager, gitu draws the first screen once the renderer has drawn it,
/// as less does with a pipeline, and the rest once the renderer has drawn that.
#[test]
fn the_first_screen_is_drawn_before_the_renderer_finishes() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str, n: usize| {
        (1..=n)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "a.rs", &lines("old a", 60));
    commit(&ctx.dir, "b.rs", "old b\n");
    fs::write(ctx.dir.join("a.rs"), lines("new a", 60)).unwrap();
    fs::write(ctx.dir.join("b.rs"), "new b\n").unwrap();
    ctx.config().general.diff_renderer.command = pausing_renderer();

    let started = Instant::now();
    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);

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

    while !draws_lines_of(&app, "b.rs") {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the rest never arrived"
        );
        std::thread::sleep(Duration::from_millis(50));
        app.update(&mut ctx.term, None).unwrap();
    }
}

/// The view is up, and can be quit, while a renderer slow to start has drawn
/// nothing; the first screen arrives once it is drawn, before the rest is.
#[test]
fn a_renderer_slow_to_start_does_not_hold_up_the_view() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str, n: usize| {
        (1..=n)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "a.rs", &lines("old a", 60));
    commit(&ctx.dir, "b.rs", "old b\n");
    fs::write(ctx.dir.join("a.rs"), lines("new a", 60)).unwrap();
    fs::write(ctx.dir.join("b.rs"), "new b\n").unwrap();
    ctx.config().general.diff_renderer.command = slow_to_start(pausing_renderer());

    let started = Instant::now();
    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);

    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
    assert!(!draws_lines_of(&app, "a.rs"));

    while !draws_lines_of(&app, "a.rs") {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the first screen never arrived"
        );
        std::thread::sleep(Duration::from_millis(50));
        app.update(&mut ctx.term, None).unwrap();
    }
    assert!(
        !draws_lines_of(&app, "b.rs"),
        "waited for the rest before showing the first screen"
    );
}

/// `--quit-if-one-screen` decides on all of the output, however slow the
/// renderer is to start drawing it.
#[test]
fn a_patch_that_fits_is_printed_however_slow_the_renderer() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = slow_to_start(annotating_renderer());

    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);
    ctx.term.clear().unwrap();
    ctx.term.queue_move_cursor(0, 0).unwrap();

    assert!(app.print_if_one_screen(&mut ctx.term).unwrap());
    assert!(ctx.physical_screen().contains("+changed"));
}

/// `renderer`, an awk script, drawing nothing for its first two seconds.
fn slow_to_start(renderer: Vec<String>) -> Vec<String> {
    renderer
        .into_iter()
        .map(|arg| arg.replacen("BEGIN{", "BEGIN{system(\"sleep 2\")}\nBEGIN{", 1))
        .collect()
}

/// [`annotating_renderer`], drawing its first 80 rows and then pausing for
/// three seconds before the rest.
pub(super) fn pausing_renderer() -> Vec<String> {
    annotating_renderer()
        .into_iter()
        .map(|arg| arg.replace("BEGIN{", "NR==80{fflush(); system(\"sleep 3\")}\nBEGIN{"))
        .collect()
}

pub(super) fn draws_lines_of(app: &App, file: &str) -> bool {
    app.screen().items().iter().any(|item| {
        matches!(&item.data, crate::item_data::ItemData::HunkLine { diff, file_i, .. }
            if diff.file_diffs[*file_i].header.new_file.fmt(&diff.text) == file)
    })
}

/// The colour of the cell where `text` is first drawn.
fn colour_of(ctx: &TestContext, text: &str) -> Option<crate::style::Color> {
    let crate::term::TermBackend::Test { buffer, .. } = &ctx.term else {
        unreachable!();
    };
    let screen = ctx.physical_screen();
    let row = screen.lines().position(|row| row.contains(text))?;
    let column = screen.lines().nth(row)?.find(text)?;
    let column = screen.lines().nth(row)?[..column].chars().count();
    Some(buffer.cells[row * buffer.width as usize + column].fg)
}

/// The colours a commit's format asks for are what the user sees, as they are
/// with `git show` in a terminal: put again, the command is asked for colour,
/// and the patch is parsed without it.
#[test]
fn the_colours_a_commits_format_asks_for_are_kept() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");

    let mut app = ctx.init_app_as_pager_of(&["git", "show", "--format=%C(blue)%an%C(reset)"]);

    assert_eq!(
        colour_of(&ctx, "Author Name"),
        Some(crate::style::Color::Indexed(4)),
        "{}",
        ctx.redact_buffer()
    );
    ctx.update(&mut app, keys("<ctrl+j>"));
    assert!(matches!(
        app.screen().get_selected_item().data,
        crate::item_data::ItemData::HunkLine { .. } | crate::item_data::ItemData::Hunk { .. }
    ));
}

/// A piped patch keeps the colours it arrived with above its diff too.
#[test]
fn a_piped_patch_keeps_the_colours_of_its_commit() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let patch = run(
        &ctx.dir,
        &[
            "git",
            "show",
            "--color=always",
            "--format=%C(blue)%an%C(reset)",
        ],
    );

    ctx.init_app_with_patch(patch);

    assert_eq!(
        colour_of(&ctx, "Author Name"),
        Some(crate::style::Color::Indexed(4)),
        "{}",
        ctx.redact_buffer()
    );
}

/// A re-render keeps the cursor on its diff line though that line is drawn only
/// after the first screenfuls: it is there when the user gets control back, not
/// moved there later.
#[test]
fn a_rerender_keeps_the_cursor_on_a_line_drawn_late() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str, n: usize| {
        (1..=n)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "a.rs", &lines("old a", 60));
    commit(&ctx.dir, "b.rs", "old b\n");
    fs::write(ctx.dir.join("a.rs"), lines("new a", 60)).unwrap();
    fs::write(ctx.dir.join("b.rs"), "new b\n").unwrap();
    ctx.config().general.diff_renderer.command = pausing_renderer();
    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);
    let started = Instant::now();
    while app.screen().is_drawing() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "never finished"
        );
        std::thread::sleep(Duration::from_millis(50));
        app.update(&mut ctx.term, None).unwrap();
    }
    ctx.update(&mut app, keys("/new b<enter>"));
    assert_eq!(selected_line_of(&app), Some(("b.rs".into(), 1)));

    ctx.update(&mut app, keys("H<enter>"));
    assert_eq!(selected_line_of(&app), Some(("b.rs".into(), 1)));
}

fn selected_line_of(app: &App) -> Option<(String, usize)> {
    match &app.screen().get_selected_item().data {
        crate::item_data::ItemData::HunkLine {
            diff,
            file_i,
            line_i,
            ..
        } => Some((
            diff.file_diffs[*file_i]
                .header
                .new_file
                .fmt(&diff.text)
                .to_string(),
            *line_i,
        )),
        _ => None,
    }
}

/// Hiding a file is kill-line: the cursor stays on the row that was below it,
/// which is the next file, whether files are folded to their stat rows or not.
#[test]
fn hiding_a_file_leaves_the_cursor_on_the_next_file() {
    for (args, to_second_file) in [
        (&["git", "diff"][..], "k<alt+j>"),
        (&["git", "diff", "--stat"], "j"),
    ] {
        let mut ctx = setup_clone!();
        three_changed_files(&ctx);

        let mut app = ctx.init_app_as_pager_of(args);
        ctx.update(&mut app, keys(to_second_file));
        assert_eq!(selected_file(&app).as_deref(), Some("b.rs"), "{args:?}");

        ctx.update(&mut app, keys("d"));

        assert_eq!(
            selected_file(&app).as_deref(),
            Some("c.rs"),
            "{args:?}\n{}",
            ctx.redact_buffer()
        );
    }
}

fn three_changed_files(ctx: &TestContext) {
    for file in ["a.rs", "b.rs", "c.rs"] {
        commit(&ctx.dir, file, "before\n");
        fs::write(ctx.dir.join(file), "after\n").unwrap();
    }
}

fn selected_file(app: &App) -> Option<String> {
    match &app.screen().get_selected_item().data {
        crate::item_data::ItemData::Delta { diff, file_i, .. } => Some(
            diff.file_diffs[*file_i]
                .header
                .new_file
                .fmt(&diff.text)
                .into_owned(),
        ),
        _ => None,
    }
}

/// ... and so does hiding a file whose diff is drawn only after the first
/// screenfuls: the rest of the render arriving does not move the cursor.
#[test]
fn hiding_a_file_drawn_late_leaves_the_cursor_on_the_next_file() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str| {
        (1..=60)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    for file in [
        "a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs",
    ] {
        commit(&ctx.dir, file, &lines("old"));
        fs::write(ctx.dir.join(file), lines("new")).unwrap();
    }
    ctx.config().general.diff_renderer.command = pausing_renderer();
    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff", "--stat"]);
    finish_drawing(&mut ctx, &mut app);
    ctx.update(&mut app, keys("jjjj"));
    assert_eq!(selected_file(&app).as_deref(), Some("e.rs"));

    ctx.update(&mut app, keys("d"));
    finish_drawing(&mut ctx, &mut app);

    assert_eq!(selected_file(&app).as_deref(), Some("f.rs"));
}

/// Hiding the last file leaves the cursor on the file above it, as kill-line
/// on the last line does, however late the render reaches it.
#[test]
fn hiding_the_last_file_leaves_the_cursor_on_the_file_above() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str| {
        (1..=60)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    for file in [
        "a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "g.rs", "h.rs",
    ] {
        commit(&ctx.dir, file, &lines("old"));
        fs::write(ctx.dir.join(file), lines("new")).unwrap();
    }
    ctx.config().general.diff_renderer.command = pausing_renderer();
    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff", "--stat"]);
    finish_drawing(&mut ctx, &mut app);
    ctx.update(&mut app, keys("jjjjjjj"));
    assert_eq!(selected_file(&app).as_deref(), Some("h.rs"));

    ctx.update(&mut app, keys("d"));
    finish_drawing(&mut ctx, &mut app);

    assert_eq!(selected_file(&app).as_deref(), Some("g.rs"));
}

fn finish_drawing(ctx: &mut TestContext, app: &mut App) {
    let started = Instant::now();
    while app.screen().is_drawing() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "never finished"
        );
        std::thread::sleep(Duration::from_millis(50));
        app.update(&mut ctx.term, None).unwrap();
    }
}

/// Two files changed in the working tree: one line replaced in `a.rs`, three
/// lines added to `longer_b.rs`.
fn a_replaced_line_and_three_added(ctx: &TestContext) {
    commit(&ctx.dir, "a.rs", "keep\nold\n");
    commit(&ctx.dir, "longer_b.rs", "keep\n");
    fs::write(ctx.dir.join("a.rs"), "keep\nnew\n").unwrap();
    fs::write(ctx.dir.join("longer_b.rs"), "keep\n1\n2\n3\n").unwrap();
}

fn folded_to_stat_rows(ctx: &mut TestContext) -> String {
    ctx.config().general.file_header = crate::config::FileHeader::Stat;
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("<backtab>"));
    ctx.redact_buffer()
}

/// A folded file shows its line of a diffstat, aligned with the others as git
/// aligns them, when that is the style asked for.
#[test]
fn a_folded_file_shows_its_stat_row() {
    let mut ctx = setup_clone!();
    a_replaced_line_and_three_added(&ctx);

    let buffer = folded_to_stat_rows(&mut ctx);

    assert!(buffer.contains("a.rs        | 2 +-"), "{buffer}");
    assert!(buffer.contains("longer_b.rs | 3 +++"), "{buffer}");
}

#[test]
fn a_folded_file_shows_its_stat_row_whatever_drew_it() {
    let mut ctx = setup_clone!();
    a_replaced_line_and_three_added(&ctx);

    let buffer = folded_to_stat_rows(&mut ctx);

    assert!(buffer.contains("a.rs        | 2 +-"), "{buffer}");
    assert!(buffer.contains("longer_b.rs | 3 +++"), "{buffer}");
}

/// The stat row heads the file unfolded too, in place of the renderer's header.
#[test]
fn an_unfolded_file_is_headed_by_its_stat_row() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = file_header_renderer();
    ctx.config().general.file_header = crate::config::FileHeader::Stat;

    ctx.init_app_as_pager_of(&["git", "diff"]);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("firstfile | 2 +-"), "{buffer}");
    assert!(buffer.contains("changed"), "{buffer}");
    assert!(!buffer.contains("~ firstfile ~"), "{buffer}");
    assert!(!buffer.contains("==========="), "{buffer}");
}

/// Stat rows are laid out as git lays out `--stat` at the width they have:
/// long names shortened from the left, bars scaled to fit, and a summary.
#[test]
fn stat_rows_are_laid_out_as_git_lays_them_out() {
    let mut ctx = setup_clone!();
    let long = "a/very/long/directory/path/that/keeps/going/and/going/file_with_a_long_name.rs";
    fs::create_dir_all(ctx.dir.join(long).parent().unwrap()).unwrap();
    commit(&ctx.dir, "big.rs", "");
    commit(&ctx.dir, "small.rs", "");
    commit(&ctx.dir, long, "");
    let lines = |n: usize| (1..=n).map(|i| format!("{i}\n")).collect::<String>();
    fs::write(ctx.dir.join("big.rs"), lines(100)).unwrap();
    fs::write(ctx.dir.join("small.rs"), lines(1)).unwrap();
    fs::write(ctx.dir.join(long), lines(7)).unwrap();

    let buffer = folded_to_stat_rows(&mut ctx);

    let git = run(&ctx.dir, &["sh", "-c", "COLUMNS=78 git diff --stat"]);
    for line in git.lines() {
        assert!(buffer.contains(line), "{line:?} not in\n{buffer}");
    }
}

#[test]
fn a_deleted_files_stat_row_shows_its_path() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    fs::remove_file(ctx.dir.join("a.rs")).unwrap();

    let buffer = folded_to_stat_rows(&mut ctx);

    assert!(buffer.contains("a.rs | 1 -"), "{buffer}");
    assert!(!buffer.contains("/dev/null"), "{buffer}");
}

#[test]
fn a_folded_file_shows_the_renderers_header_by_default() {
    let mut ctx = setup_clone!();
    a_replaced_line_and_three_added(&ctx);
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("<backtab>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs"), "{buffer}");
    assert!(!buffer.contains(" | "), "{buffer}");
}

/// Every view of a patch can open with its files folded, however it was opened.
#[test]
fn files_start_folded_when_asked() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    taller_than_the_screen(&ctx);
    ctx.config().general.start_folded = true;

    ctx.init_app_as_pager_of(&["git", "diff"]);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs"), "{buffer}");
    assert!(buffer.contains("a_test.rs"), "{buffer}");
    assert!(!buffer.contains("alpha changed"), "{buffer}");
}

/// Two screenfuls of the renderer's rows are only a few files once they are
/// folded, so the first screen waits for enough files to fill it.
#[test]
fn files_starting_folded_fill_the_first_screen() {
    let mut ctx = setup_clone!();
    let lines = |suffix: &str| {
        (1..=20)
            .map(|i| format!("{i}{suffix}\n"))
            .collect::<String>()
    };
    for i in 1..=45 {
        commit(&ctx.dir, &format!("f{i:02}.rs"), &lines(""));
        fs::write(ctx.dir.join(format!("f{i:02}.rs")), lines(" changed")).unwrap();
    }
    ctx.config().general.start_folded = true;
    // Draws two files, then all but the last, then the last.
    ctx.config().general.diff_renderer.command = annotating_renderer()
        .into_iter()
        .map(|arg| {
            arg.replace(
                "BEGIN{",
                r#"/diff --git /{files++; if (files == 3) {fflush(); system("sleep 0.05")} if (files == 45) {fflush(); system("sleep 2")}}
        BEGIN{"#,
            )
        })
        .collect();

    ctx.init_app_streaming_as_pager_of(&["git", "diff"]);

    let buffer = ctx.redact_buffer();
    let files = buffer.lines().filter(|line| line.contains(".rs")).count();
    assert!(files >= 15, "{buffer}");
}

/// A file folded to its stat row needs nothing from the renderer, so a
/// diffstat's first screen does not wait for the renderer to reach its files.
#[test]
fn a_diffstat_is_drawn_before_the_renderer_reaches_its_files() {
    let mut ctx = setup_clone!();
    changed_files_and_a_renderer_stopping_after_two(&mut ctx, 12);

    let started = std::time::Instant::now();
    ctx.init_app_streaming_as_pager_of(&["git", "diff", "--stat"]);

    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("f12.rs"), "{buffer}");
}

/// Files `f01.rs`… changed in the worktree, and a renderer that stops for 3s
/// once it has drawn two of them.
pub(super) fn changed_files_and_a_renderer_stopping_after_two(ctx: &mut TestContext, count: usize) {
    for i in 1..=count {
        commit(&ctx.dir, &format!("f{i:02}.rs"), "old\n");
        fs::write(ctx.dir.join(format!("f{i:02}.rs")), "new\n").unwrap();
    }
    ctx.config().general.diff_renderer.command = annotating_renderer()
        .into_iter()
        .map(|arg| {
            arg.replace(
                "BEGIN{",
                r#"/diff --git /{files++; if (files == 3) {fflush(); system("sleep 3")}}
        BEGIN{"#,
            )
        })
        .collect();
}

#[test]
fn a_commit_opened_from_the_log_starts_folded_when_asked() {
    let mut ctx = setup_clone!();
    let tall = (1..=30).map(|i| format!("{i}\n")).collect::<String>();
    fs::write(ctx.dir.join("secondfile"), tall).unwrap();
    run(&ctx.dir, &["git", "add", "secondfile"]);
    commit(&ctx.dir, "firstfile", "This should be folded\n");
    ctx.config().general.start_folded = true;
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("ll<enter>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("firstfile"), "{buffer}");
    assert!(!buffer.contains("This should be folded"), "{buffer}");
}

#[test]
fn files_start_folded_to_their_stat_rows() {
    let mut ctx = setup_clone!();
    a_replaced_line_and_three_added(&ctx);
    taller_than_the_screen(&ctx);
    ctx.config().general.start_folded = true;
    ctx.config().general.file_header = crate::config::FileHeader::Stat;

    ctx.init_app_as_pager_of(&["git", "diff"]);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("a.rs        |  2 +-"), "{buffer}");
    assert!(buffer.contains("longer_b.rs |  3 +++"), "{buffer}");
}

/// Unfolding everything at once heads the files with the renderer's headers,
/// stat rows or not; folding everything again folds them to their stat rows.
#[test]
fn unfolding_everything_heads_the_files_with_the_renderers_headers() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = file_header_renderer();
    ctx.config().general.file_header = crate::config::FileHeader::Stat;
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("<backtab><backtab>"));
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("~ firstfile ~"), "{buffer}");
    assert!(buffer.contains("changed"), "{buffer}");
    assert!(!buffer.contains("firstfile | 2 +-"), "{buffer}");

    ctx.update(&mut app, keys("<backtab>"));
    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("firstfile | 2 +-"), "{buffer}");
    assert!(!buffer.contains("~ firstfile ~"), "{buffer}");
}

#[test]
fn unfolding_everything_from_a_diffstat_heads_the_files_with_the_renderers_headers() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();
    ctx.config().general.diff_renderer.command = file_header_renderer();
    let mut app = ctx.init_app_as_pager_of(&["git", "diff", "--stat"]);

    ctx.update(&mut app, keys("<backtab>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("~ firstfile ~"), "{buffer}");
    assert!(buffer.contains("changed"), "{buffer}");
    assert!(!buffer.contains("firstfile | 2 +-"), "{buffer}");
}

/// A file the user unfolded stays unfolded through a re-render, such as
/// turning on line numbers, though files start folded, and though the renderer
/// draws nothing before the first screen is shown.
#[test]
fn an_unfolded_file_stays_unfolded_through_a_slow_rerender() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    taller_than_the_screen(&ctx);
    ctx.config().general.start_folded = true;
    let renderer = &mut ctx.config().general.diff_renderer;
    renderer.features = vec!["line-numbers".into()];
    renderer.command = annotating_renderer()
        .into_iter()
        .map(|arg| arg.replace("BEGIN{", "BEGIN{system(\"sleep 1\");"))
        .collect();
    let mut app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);
    finish_drawing(&mut ctx, &mut app);
    ctx.update(&mut app, keys("gg<tab>"));
    assert!(ctx.redact_buffer().contains("alpha changed"));

    ctx.update(&mut app, keys("H<enter>"));
    finish_drawing(&mut ctx, &mut app);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha changed"), "{buffer}");
    assert!(!buffer.contains("beta changed"), "{buffer}");
}

/// A file a rebuild brings that the view has not had before starts folded, as
/// the view's first files did.
#[test]
fn a_file_new_to_a_rebuild_starts_folded() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "b.rs", "beta\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    taller_than_the_screen(&ctx);
    ctx.config().general.start_folded = true;
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    fs::write(ctx.dir.join("b.rs"), "beta changed\n").unwrap();
    ctx.update(&mut app, keys("gr"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("b.rs"), "{buffer}");
    assert!(!buffer.contains("beta changed"), "{buffer}");
}

/// A diffstat is the diff with its files headed by their stat rows and folded,
/// drawn as any diff is.
#[test]
fn a_diffstat_is_the_diff_folded_to_stat_rows() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    taller_than_the_screen(&ctx);
    ctx.config().general.diff_renderer.command = file_header_renderer();
    let stat = {
        ctx.init_app_as_pager_of(&["git", "diff", "--stat"]);
        ctx.redact_buffer()
    };
    ctx.config().general.start_folded = true;
    ctx.config().general.file_header = crate::config::FileHeader::Stat;

    ctx.init_app_as_pager_of(&["git", "diff"]);

    assert_eq!(stat, ctx.redact_buffer());
}

/// While the renderer has drawn the files' headers but none of their lines,
/// what is shown is the renderer's drawing so far, not gitu's own drawing of
/// the patch in its place.
#[test]
fn a_patch_part_drawn_is_shown_as_the_renderer_drew_it() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    ctx.config().general.diff_renderer.command = annotating_renderer()
        .into_iter()
        .map(|arg| arg.replace("BEGIN{", "NR==6{fflush(); system(\"sleep 2\")}\nBEGIN{"))
        .collect();

    let app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);

    let undrawn: Vec<_> = app
        .screen()
        .items()
        .iter()
        .filter(|item| matches!(item.data, crate::item_data::ItemData::HunkLine { .. }))
        .filter(|item| item.rendered.is_none())
        .collect();
    assert!(undrawn.is_empty(), "{undrawn:?}");
}

/// A view the renderer has part drawn has only the files it has reached: a
/// file it has not drawn yet is not shown in some other form until it has.
#[test]
fn a_part_drawn_patch_has_only_the_files_drawn_so_far() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    ctx.config().general.diff_renderer.command = annotating_renderer()
        .into_iter()
        .map(|arg| {
            arg.replace(
                "BEGIN{",
                "/diff --git/ && ++files == 2 {fflush(); system(\"sleep 2\")}\nBEGIN{",
            )
        })
        .collect();

    let app = ctx.init_app_streaming_as_pager_of(&["git", "diff"]);

    let files: Vec<usize> = app
        .screen()
        .items()
        .iter()
        .filter_map(|item| match item.data {
            crate::item_data::ItemData::Delta { file_i, .. } => Some(file_i),
            _ => None,
        })
        .collect();
    assert_eq!(files, vec![0]);
}

/// A patch of one file has nothing to fold it away from, so it opens unfolded.
#[test]
fn a_patch_of_one_file_starts_unfolded() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "a.rs", "alpha\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    ctx.config().general.start_folded = true;

    ctx.init_app_as_pager_of(&["git", "diff"]);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha changed"), "{buffer}");
}

/// A patch that fits on one screen is all in view already, so it opens
/// unfolded.
#[test]
fn a_patch_that_fits_on_one_screen_starts_unfolded() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);
    ctx.config().general.start_folded = true;

    ctx.init_app_as_pager_of(&["git", "diff"]);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("alpha changed"), "{buffer}");
    assert!(buffer.contains("beta changed"), "{buffer}");
}

/// A view that opens with its files folded puts the cursor on the first of
/// them, ready to unfold the one wanted.
#[test]
fn a_view_opening_folded_starts_on_its_first_file() {
    let mut ctx = setup_clone!();
    let tall = (1..=30).map(|i| format!("{i}\n")).collect::<String>();
    fs::write(ctx.dir.join("secondfile"), tall).unwrap();
    run(&ctx.dir, &["git", "add", "secondfile"]);
    commit(&ctx.dir, "firstfile", "first\n");
    ctx.config().general.start_folded = true;
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("ll<enter>"));

    assert!(
        matches!(
            app.screen().get_selected_item().data,
            crate::item_data::ItemData::Delta { file_i: 0, .. }
        ),
        "{:?}\n{}",
        app.screen().get_selected_item().data,
        ctx.redact_buffer()
    );
}

/// A path in the command is relative to where git ran it.
#[test]
fn a_command_run_in_a_subdirectory_is_run_there_again() {
    let mut ctx = setup_clone!();
    fs::create_dir(ctx.dir.join("sub")).unwrap();
    commit(&ctx.dir, "sub/lib.rs", "sub\n");
    commit(&ctx.dir, "lib.rs", "top\n");
    fs::write(ctx.dir.join("sub/lib.rs"), "sub changed\n").unwrap();
    fs::write(ctx.dir.join("lib.rs"), "top changed\n").unwrap();

    ctx.init_app_as_pager_of_in(ctx.dir.join("sub"), &["git", "diff", "--", "lib.rs"]);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("sub changed"), "{buffer}");
    assert!(!buffer.contains("top changed"), "{buffer}");
}

/// As in less, j, k and the arrow keys move through a diff a line at a time.
#[test]
fn j_k_and_the_arrow_keys_move_a_line_at_a_time() {
    for (down, up) in [("j", "k"), ("<down>", "<up>")] {
        let mut ctx = setup_clone!();
        commit(&ctx.dir, "firstfile", "a\nb\nc\n");
        fs::write(ctx.dir.join("firstfile"), "A\nB\nc\n").unwrap();
        let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

        ctx.update(&mut app, keys(down));
        let first = selected_hunk_line(&app);
        assert!(first.is_some(), "{}", ctx.redact_buffer());
        ctx.update(&mut app, keys(down));
        assert_eq!(selected_hunk_line(&app), first.map(|line_i| line_i + 1));
        ctx.update(&mut app, keys(up));
        assert_eq!(selected_hunk_line(&app), first);
    }
}

/// With ctrl, j and k move past a hunk's lines, to the next hunk.
#[test]
fn ctrl_j_moves_to_the_next_hunk() {
    let mut ctx = setup_clone!();
    let lines = |second: &str, last: &str| {
        let mut lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        lines[1] = second.to_owned();
        lines[28] = last.to_owned();
        lines.join("\n") + "\n"
    };
    commit(&ctx.dir, "firstfile", &lines("line 2", "line 29"));
    fs::write(ctx.dir.join("firstfile"), lines("changed 2", "changed 29")).unwrap();
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys("<ctrl+j>"));

    assert!(
        matches!(
            app.screen().get_selected_item().data,
            crate::item_data::ItemData::Hunk { hunk_i: 1, .. }
        ),
        "{:?}\n{}",
        app.screen().get_selected_item().data,
        ctx.redact_buffer()
    );
}
