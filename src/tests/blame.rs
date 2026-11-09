use super::*;

fn setup(ctx: TestContext) -> TestContext {
    commit(
        &ctx.dir,
        "file",
        "unchanged-top\noriginal\nunchanged-bottom\n",
    );
    commit(
        &ctx.dir,
        "file",
        "unchanged-top\nSELECT-THIS-LINE\nunchanged-bottom\n",
    );
    ctx
}

fn setup_shifted(ctx: TestContext) -> TestContext {
    // commit A: file with target line at position 2
    commit(&ctx.dir, "file", "top\nSELECT-THIS-LINE\nbottom\n");
    // commit B: prepends 5 lines, pushing SELECT-THIS-LINE to position 7 in HEAD
    commit(
        &ctx.dir,
        "file",
        "added1\nadded2\nadded3\nadded4\nadded5\ntop\nSELECT-THIS-LINE\nbottom\n",
    );
    ctx
}

#[test]
fn blame_from_hunk() {
    snapshot!(setup(setup_clone!()), "ll<enter>B");
}

#[test]
fn blame_navigate_code_line() {
    snapshot!(setup(setup_clone!()), "ll<enter>B<ctrl+j>");
}

#[test]
fn blame_enter_on_header() {
    snapshot!(setup(setup_clone!()), "ll<enter>Bk<enter>");
}

#[test]
fn blame_enter_on_code_line() {
    snapshot!(setup(setup_clone!()), "ll<enter>B<enter>");
}

#[test]
fn blame_re_blame_from_header() {
    snapshot!(setup(setup_clone!()), "ll<enter>BB");
}

#[test]
fn blame_re_blame_from_code_line() {
    snapshot!(setup(setup_clone!()), "ll<enter>BB");
}

#[test]
fn blame_from_hunk_shifted_lines() {
    snapshot!(setup_shifted(setup_clone!()), "llj<enter>B");
}

/// Blame is drawn by the renderer, as git blame's output is by delta, one row
/// per line, in place of gitu's own drawing with a header row per commit.
#[test]
fn blame_lines_are_drawn_by_the_renderer() {
    let mut ctx = setup(setup_clone!());
    // The usual test renderer, with each line of git blame's output marked.
    let awk = crate::tests::pager::annotating_renderer()[1].clone();
    ctx.config().general.diff_renderer.command = vec![
        "sh".into(),
        "-c".into(),
        r#"awk "$1" | sed -E '/[0-9a-f]{7,} \(/s/$/ <blame row/'"#.into(),
        "sh".into(),
        awk,
    ];
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("ll<enter>B"));

    let screen = ctx.redact_buffer();
    let rows: Vec<&str> = screen
        .lines()
        .filter(|row| row.contains("<blame row"))
        .collect();
    assert_eq!(rows.len(), 3, "{screen}");
    assert!(rows[1].contains("SELECT-THIS-LINE"), "{screen}");
    assert!(!screen.contains("modify file"), "{screen}");
}
