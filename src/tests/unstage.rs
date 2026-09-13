use super::*;

#[test]
fn unstage_all_staged() {
    let ctx = setup_clone!();
    run(&ctx.dir, &["touch", "one", "two", "unaffected"]);
    run(&ctx.dir, &["git", "add", "one", "two"]);
    snapshot!(ctx, "jjju");
}

#[test]
fn unstage_removed_line() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();
    run(&ctx.dir, &["git", "add", "."]);
    snapshot!(ctx, "jj<tab><ctrl+j><ctrl+j>u");
}

#[test]
fn unstage_added_line() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();
    run(&ctx.dir, &["git", "add", "."]);
    snapshot!(ctx, "jj<tab><ctrl+j><ctrl+j><ctrl+j><ctrl+j>u");
}

#[test]
fn unstage_deleted_file() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "to-delete", "testing\ntesttest\n");
    run(&ctx.dir, &["git", "rm", "to-delete"]);
    snapshot!(ctx, "jju");
}

#[test]
#[cfg(not(target_os = "windows"))]
fn unstage_deleted_executable_file() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "script.sh", "#!/bin/bash\necho hello\n");
    run(&ctx.dir, &["chmod", "+x", "script.sh"]);
    run(&ctx.dir, &["git", "add", "script.sh"]);
    run(&ctx.dir, &["git", "commit", "-m", "add executable script"]);
    run(&ctx.dir, &["git", "rm", "script.sh"]);
    snapshot!(ctx, "jju");
}

#[test]
fn unstage_added_file_with_spaces_in_name() {
    let ctx = setup_clone!();
    run(&ctx.dir, &["touch", "file with space.txt"]);
    run(&ctx.dir, &["git", "add", "file with space.txt"]);
    snapshot!(ctx, "jju");
}

/// A selection unstages as one patch too, the reverse of staging one.
#[test]
fn unstage_selected_lines() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\ntesttest\n");
    fs::write(ctx.dir.join("firstfile"), "weehooo\nblrergh\n").unwrap();
    run(&ctx.dir, &["git", "add", "."]);

    let mut app = ctx.init_app();
    ctx.update(&mut app, keys("jj<tab><ctrl+j><ctrl+j><shift+down>u"));

    // Both removals are put back, leaving only the two additions staged.
    assert_eq!(
        run(&ctx.dir, &["git", "show", ":firstfile"]),
        "testing\ntesttest\nweehooo\nblrergh\n"
    );
}
