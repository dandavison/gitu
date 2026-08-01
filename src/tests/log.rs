use super::*;

fn setup(ctx: TestContext) -> TestContext {
    commit(&ctx.dir, "third commit", "");
    commit(&ctx.dir, "second commit", "");
    commit(&ctx.dir, "first commit", "");
    ctx
}

#[test]
fn limit_prompt() {
    snapshot!(setup(setup_clone!()), "l-n-n");
}

#[test]
fn limit_set_10() {
    snapshot!(setup(setup_clone!()), "l-n-n10<enter>");
}

#[test]
fn limit_invalid() {
    snapshot!(setup(setup_clone!()), "l-n-nfff<enter>");
}

#[test]
fn limit_2_commits() {
    snapshot!(setup(setup_clone!()), "l-n-n2<enter>l");
}

#[test]
fn limit_2_commits_other() {
    snapshot!(setup(setup_clone!()), "l-n-n2<enter>l");
}

#[test]
fn grep_prompt() {
    snapshot!(setup(setup_clone!()), "l-F");
}

#[test]
fn grep_set_example() {
    snapshot!(setup(setup_clone!()), "l-Fexample<enter>");
}

#[test]
fn grep_second() {
    snapshot!(setup(setup_clone!()), "l-Fsecond<enter>l");
}

#[test]
fn grep_no_match() {
    snapshot!(setup(setup_clone!()), "l-Fdoesntexist<enter>l");
}

#[test]
fn grep_second_other() {
    snapshot!(setup(setup_clone!()), "l-Fsecond<enter>omain<enter>");
}

#[test]
fn log_other_prompt() {
    snapshot!(setup(setup_clone!()), "lljlo");
}

#[test]
fn log_other() {
    snapshot!(setup(setup_clone!()), "lljlo<enter>");
}

#[test]
fn log_other_input() {
    snapshot!(setup(setup_clone!()), "lomain~1<enter>");
}

#[test]
fn log_other_invalid() {
    snapshot!(setup(setup_clone!()), "lo <enter>");
}

#[test]
fn log_empty_branch() {
    // Regression for #262: showing the log of a branch with no commits used to
    // panic ("index out of bounds") because the log screen had no items but the
    // cursor still indexed into it.
    let mut ctx = setup_clone!();
    run(&ctx.dir, &["rm", "-rf", ".git"]);
    run(&ctx.dir, &["rm", "initial-file"]);
    run(&ctx.dir, &["git", "init", "--initial-branch=main"]);

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

/// Render the log with a command whose format prints each commit over several
/// rows, marked with the `{commit}` token so gitu can tell the rows apart. Dates
/// are absolute so the output doesn't drift with the wall clock.
pub(super) fn with_log_renderer(mut ctx: TestContext, args: &[&str], format: &str) -> TestContext {
    let config = ctx.config();
    config.general.log_renderer.enabled = true;
    config.general.log_renderer.command = [&["git", "log", "--color=always", "--date=short"], args]
        .concat()
        .iter()
        .map(|arg| arg.to_string())
        .chain([format!("--format={format}")])
        .collect();
    ctx
}

pub(super) const MULTI_ROW_FORMAT: &str =
    "{commit}%n\u{25b8} %h %an %ad%C(auto)%d%C(reset)%n    %s";

#[test]
fn rendered_log() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "ll"
    );
}

#[test]
fn rendered_log_move_down_steps_a_whole_commit() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "lljj"
    );
}

#[test]
fn rendered_log_move_next_section_steps_a_whole_commit() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "ll<alt+j><alt+j>"
    );
}

#[test]
fn rendered_log_move_prev_section_steps_a_whole_commit() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "lljj<alt+k>"
    );
}

#[test]
fn rendered_log_fold_hides_the_commit_rows() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "ll<tab>"
    );
}

#[test]
fn rendered_log_show_acts_on_the_selected_commit() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "llj<enter>"
    );
}

#[test]
fn rendered_log_copy_hash_of_the_selected_commit() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "lljy"
    );
}

#[test]
fn rendered_log_with_stat() {
    // Extra rows a commit prints after its message stay part of that commit.
    snapshot!(
        with_log_renderer(
            setup(setup_clone!()),
            &["--stat"],
            "{commit}%n\u{25b8} %h %s"
        ),
        "llj"
    );
}

#[test]
fn rendered_log_limit() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "l-n-n2<enter>l"
    );
}

#[test]
fn rendered_log_grep() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "l-Fsecond<enter>l"
    );
}

#[test]
fn rendered_log_other_rev() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], MULTI_ROW_FORMAT),
        "lomain~1<enter>"
    );
}

#[test]
fn rendered_log_without_commit_marker_falls_back() {
    snapshot!(with_log_renderer(setup(setup_clone!()), &[], "%h %s"), "ll");
}

#[test]
fn rendered_log_empty_branch_falls_back() {
    // The log command exits non-zero on an unborn branch; gitu falls back rather
    // than showing nothing.
    let ctx = setup_clone!();
    run(&ctx.dir, &["rm", "-rf", ".git"]);
    run(&ctx.dir, &["rm", "initial-file"]);
    run(&ctx.dir, &["git", "init", "--initial-branch=main"]);
    let mut ctx = with_log_renderer(ctx, &[], MULTI_ROW_FORMAT);

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    insta::assert_snapshot!(ctx.redact_buffer());
}

/// A renderer that draws a rule above each commit, as delta does with
/// `commit-decoration-style = ol`.
const DECORATED_FORMAT: &str = "{commit}%n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}%n\u{25b8} %h %s";

#[test]
fn rendered_log_divider_is_not_the_cursor_line() {
    snapshot!(
        with_log_renderer(setup(setup_clone!()), &[], DECORATED_FORMAT),
        "llj"
    );
}

#[test]
fn rendered_log_keeps_the_first_commit_when_the_renderer_greets_on_its_line() {
    // A renderer emits its OSC-1717 handshake as its first output, which shares
    // a line with the marker a format puts at the very start (delta does this).
    // The newest commit must still be a commit: selectable, cursor on it.
    let mut ctx = setup(setup_clone!());
    ctx.config().general.log_renderer.enabled = true;
    ctx.config().general.log_renderer.command = [
        "sh",
        "-c",
        r#"printf '\033]1717;1\033\\'; git log --format="{commit}%n───%n▸ %h %s" "$@""#,
        "gitu",
    ]
    .map(String::from)
    .to_vec();

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll"));
    insta::assert_snapshot!(ctx.redact_buffer());
}
