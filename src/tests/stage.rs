use super::*;

#[test]
fn staged_file() {
    let mut ctx = setup_clone!();
    run(&ctx.dir, &["touch", "new-file"]);
    run(&ctx.dir, &["git", "add", "new-file"]);

    ctx.init_app();
    insta::assert_snapshot!(ctx.redact_buffer());
}

#[test]
fn stage_all_unstaged() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    commit(&ctx.dir, "secondfile", "testing\ntesttest\n");

    fs::write(ctx.dir.join("firstfile"), "blahonga\n").unwrap();
    fs::write(ctx.dir.join("secondfile"), "blahonga\n").unwrap();
    snapshot!(ctx, "js");
}

#[test]
fn stage_all_untracked() {
    let ctx = setup_clone!();
    run(&ctx.dir, &["touch", "file-a"]);
    run(&ctx.dir, &["touch", "file-b"]);
    snapshot!(ctx, "js");
}

#[test]
fn stage_removed_line() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();
    snapshot!(ctx, "jj<tab><ctrl+j><ctrl+j>s");
}

#[test]
fn stage_added_line() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();

    snapshot!(ctx, "jj<tab><ctrl+j><ctrl+j><ctrl+j><ctrl+j>s");
}

#[test]
fn stage_changes_crlf() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "testfile", "testing\r\ntesttest\r\n");
    fs::write(ctx.dir.join("testfile"), "test\r\ntesttest\r\n").expect("error writing to file");

    snapshot!(ctx, "jj<tab>");
}

#[test]
fn stage_deleted_file() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "to-delete", "testing\ntesttest\n");
    run(&ctx.dir, &["rm", "to-delete"]);
    snapshot!(ctx, "jjs");
}

/// Extending the selection over several lines stages all of them, as one patch.
#[test]
fn stage_selected_lines() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();

    let mut app = ctx.init_app();
    ctx.update(
        &mut app,
        keys("jj<tab><ctrl+j><ctrl+j><ctrl+j><ctrl+j><shift+down>s"),
    );

    assert_eq!(
        run(&ctx.dir, &["git", "show", ":firstfile"]),
        "testing\ntesttest\nweehooo\nblrergh\n"
    );
}

/// The lines the selection covers are marked, so it is plain what `s` will take.
#[test]
fn selected_lines_are_marked() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();
    snapshot!(ctx, "jj<tab><ctrl+j><ctrl+j><ctrl+j><ctrl+j><shift+down>");
}

/// Moving away without shift drops the selection, leaving just the cursor line.
#[test]
fn moving_on_drops_the_selection() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();

    let mut app = ctx.init_app();
    ctx.update(
        &mut app,
        keys("jj<tab><ctrl+j><ctrl+j><ctrl+j><shift+down><ctrl+j>s"),
    );

    assert_eq!(
        run(&ctx.dir, &["git", "show", ":firstfile"]),
        "testing\ntesttest\nblrergh\n"
    );
}

#[test]
#[cfg(not(target_os = "windows"))]
fn stage_deleted_executable_file() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "script.sh", "#!/bin/bash\necho hello\n");
    run(&ctx.dir, &["chmod", "+x", "script.sh"]);
    run(&ctx.dir, &["git", "add", "script.sh"]);
    run(&ctx.dir, &["git", "commit", "-m", "add executable script"]);
    run(&ctx.dir, &["rm", "script.sh"]);
    snapshot!(ctx, "jjs");
}

