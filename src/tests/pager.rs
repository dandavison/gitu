//! Which ops a piped patch admits.
//!
//! There is no such thing as staging a line of `git show HEAD`: that patch is a
//! committed change, and applying part of it to the index is cherry-picking.
//! git tells its pager nothing about the command that produced the patch, so
//! gitu finds it in the process tree; what it finds is what the patch is.

use super::*;
use crate::{app::App, ops::Op};

#[test]
fn a_working_tree_patch_admits_stage_and_not_unstage() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    // git colours what it writes to its pager; the command survives that.
    let mut app = ctx.init_app_as_pager_of(&["git", "-c", "color.diff=always", "diff"]);
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

    let mut app = ctx.init_app_as_pager_of(&["git", "diff", "--cached"]);
    ctx.update(&mut app, keys("j"));

    assert!(offers(&app, Op::Unstage));
    assert!(!offers(&app, Op::Stage));
}

#[test]
fn a_commit_patch_admits_neither() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");

    let mut app = ctx.init_app_as_pager_of(&["git", "show", "HEAD"]);
    ctx.update(&mut app, keys("j"));

    assert!(!offers(&app, Op::Stage));
    assert!(!offers(&app, Op::Unstage));
}

#[test]
fn staging_a_hunk_of_a_piped_working_tree_patch_stages_it() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "changed\ntesttest\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
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

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
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
    ctx.update(&mut app, keys("j"));
    assert!(offers(&app, Op::Stage));
    assert!(ctx.redact_buffer().contains("+again"));

    ctx.update(&mut app, keys(": --cached<enter>j"));

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
/// to have dropped one of them without reading its name off the row that says
/// what is being asked.
fn two_changed_files(ctx: &TestContext) {
    commit(&ctx.dir, "a.rs", "alpha\n");
    commit(&ctx.dir, "a_test.rs", "beta\n");
    fs::write(ctx.dir.join("a.rs"), "alpha changed\n").unwrap();
    fs::write(ctx.dir.join("a_test.rs"), "beta changed\n").unwrap();
}

/// A hidden file is not "diff not displayed": it is absent, as if the patch
/// never had it. What is left is a diff like any other.
#[test]
fn hiding_the_file_under_the_cursor_drops_it_from_the_view() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("-j"));

    let buffer = ctx.redact_buffer();
    assert!(!buffer.contains("alpha"), "{buffer}");
    assert!(buffer.contains("beta"), "{buffer}");
    assert!(offers(&app, Op::Stage));
}

/// The paths are the user's to say, in git's own language less the magic: a
/// bare pattern limits the view, `!` drops what it matches.
#[test]
fn the_files_to_show_can_be_said_by_pattern() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("_!*_test.rs<enter>"));

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
    ctx.update(&mut app, keys("-_"));
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
            "_!a_test.rs<enter>_<ctrl+u>!a.rs<enter>\
             : HEAD<enter>:<ctrl+u>git diff --cached<enter>",
        ),
    );
    drop(app);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys(":<up>"));
    assert_eq!(ctx.prompt_line(), "git diff --cached|");

    ctx.update(&mut app, keys("_<up>"));
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

    let other = ctx.dir.parent().unwrap().join("other-worktree");
    let other_arg = other.to_str().unwrap();
    run(
        &ctx.dir,
        &["git", "worktree", "add", "-b", "other", other_arg],
    );
    let mut app = ctx.init_app_as_pager_of_at(other, &["git", "diff"]);
    ctx.update(&mut app, keys(":<up>"));

    assert_eq!(ctx.prompt_line(), "git diff|");
}

/// A view with files hidden from it must not pass for the whole patch, so a
/// question that is no longer git's own says so — in one line, and only then.
#[test]
fn a_view_that_is_not_what_git_asked_for_says_what_it_is() {
    let mut ctx = setup_clone!();
    two_changed_files(&ctx);

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    assert!(
        !ctx.redact_buffer().contains("git diff"),
        "nothing to say about git's own question:\n{}",
        ctx.redact_buffer()
    );

    ctx.update(&mut app, keys("-"));

    assert!(
        ctx.redact_buffer()
            .contains("git diff -- ':(top,exclude)a.rs'"),
        "{}",
        ctx.redact_buffer()
    );
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
    ctx.config().general.diff_renderer.enabled = true;
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

    ctx.config().general.diff_renderer.enabled = true;
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

    ctx.config().general.diff_renderer.enabled = true;
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

/// A context line carries no change, so by default the cursor passes over it:
/// every line-level op needs something to act on.
#[test]
fn context_lines_are_not_visited_by_default() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "one\ntwo\nthree\n");
    fs::write(ctx.dir.join("firstfile"), "one\nCHANGED\nthree\n").unwrap();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);
    ctx.update(&mut app, keys("<ctrl+j>"));

    assert_eq!(selected_hunk_line(&app), Some(1));
}

/// Reading a patch is line-by-line, and skipping the unchanged lines makes the
/// cursor jump. `visit_context_lines` stops on all of them.
#[test]
fn context_lines_are_visited_when_configured() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "one\ntwo\nthree\n");
    fs::write(ctx.dir.join("firstfile"), "one\nCHANGED\nthree\n").unwrap();
    ctx.config().general.visit_context_lines = true;

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    for line_i in 0..4 {
        ctx.update(&mut app, keys("<ctrl+j>"));
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
    ctx.config().general.diff_renderer.enabled = true;
    ctx.config().general.diff_renderer.command = osc_1717_passthrough();

    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    for line_i in 0..4 {
        ctx.update(&mut app, keys("<ctrl+j>"));
        assert_eq!(selected_hunk_line(&app), Some(line_i));
    }
}

/// A renderer that changes nothing but speaks the protocol, so its rows carry
/// the metadata that makes them content lines rather than plain text.
fn osc_1717_passthrough() -> Vec<String> {
    [
        "awk",
        r#"
        BEGIN{printf "\033]1717;1\033\\\n"}
        /^\+\+\+ /{file=substr($0,7);print;next}
        /^--- /{print;next}
        /^@@ /{o=$2;n=$3;sub(/^-/,"",o);sub(/^\+/,"",n);split(o,a,",");split(n,b,",");old=a[1];new=b[1];print;next}
        /^\+/{printf "\033]1717;1;a;%d;;%s\033\\%s\n",new++,file,$0;next}
        /^-/{printf "\033]1717;1;d;%d;%d;%s\033\\%s\n",new,old++,file,$0;next}
        /^ /{printf "\033]1717;1;c;%d;;%s\033\\%s\n",new++,file,$0;old++;next}
        {print}
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
    ctx.config().general.diff_renderer.enabled = true;
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

    ctx.update(&mut app, keys("|<enter>"));
    let buffer = ctx.redact_buffer();
    assert!(blank_rows_at_the_bottom(&buffer) <= 2, "{buffer}");
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
