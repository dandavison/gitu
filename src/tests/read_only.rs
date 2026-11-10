use super::*;
use crate::config::ReadOnly;
use std::path::PathBuf;

fn read_only(ctx: &mut TestContext) {
    ctx.config().general.read_only = ReadOnly::On;
}

fn porcelain_status(ctx: &TestContext) -> String {
    run(&ctx.dir, &["git", "status", "--porcelain"])
}

/// A jj repository colocated with git: jj keeps its own state in `.jj` beside
/// `.git`, which a git command run behind its back can contradict.
fn colocate_jj(ctx: &TestContext) {
    fs::create_dir(ctx.dir.join(".jj")).unwrap();
    fs::write(ctx.dir.join(".jj/.gitignore"), "/*\n").unwrap();
}

/// The help menu offers no submenu whose commands change the repository, and
/// no command on the item under the cursor that does.
#[test]
fn the_help_menu_offers_nothing_that_writes() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    commit(&ctx.dir, "file", "a\n");
    fs::write(ctx.dir.join("file"), "b\n").unwrap();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("jj?"));

    let buffer = ctx.redact_buffer();
    for writes in [
        "Branch",
        "Commit",
        "Fetch",
        "Merge",
        "Remote",
        "Pull",
        "Push",
        "Rebase",
        "Reset",
        "Revert",
        "Cherry-pick",
        "Stash",
        "Stage",
        "Discard",
    ] {
        assert!(!buffer.contains(writes), "{writes}\n{buffer}");
    }
    for reads in ["Help", "Log", "Show Refs", "Show"] {
        assert!(buffer.contains(reads), "{reads}\n{buffer}");
    }
}

#[test]
fn stage_does_nothing() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    run(&ctx.dir, &["touch", "new-file"]);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("js"));

    assert_eq!(porcelain_status(&ctx), "?? new-file\n");
}

#[test]
fn discard_does_nothing() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    commit(&ctx.dir, "file", "a\n");
    fs::write(ctx.dir.join("file"), "b\n").unwrap();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("jjKy"));

    assert_eq!(fs::read_to_string(ctx.dir.join("file")).unwrap(), "b\n");
}

/// A key that opens a menu of commands that write opens nothing.
#[test]
fn the_commit_menu_does_not_open() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("c"));

    let buffer = ctx.redact_buffer();
    assert!(!buffer.contains("Commit"), "{buffer}");
}

#[test]
fn a_jj_repository_is_read_only() {
    let mut ctx = setup_clone!();
    colocate_jj(&ctx);
    run(&ctx.dir, &["touch", "new-file"]);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("js"));

    assert_eq!(porcelain_status(&ctx), "?? new-file\n");
}

#[test]
fn read_only_turned_off_writes_in_a_jj_repository() {
    let mut ctx = setup_clone!();
    ctx.config().general.read_only = ReadOnly::Off;
    colocate_jj(&ctx);
    run(&ctx.dir, &["touch", "new-file"]);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("js"));

    assert_eq!(porcelain_status(&ctx), "A  new-file\n");
}

/// A theme chosen is used for the session and not saved.
#[test]
fn a_theme_chosen_is_not_saved() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    run(&ctx.dir, &["git", "config", "delta.theme-a.dark", "true"]);
    // The repository's own git config holds the `delta-tui` section, so it is
    // where a theme chosen would be written.
    run(
        &ctx.dir,
        &["git", "config", "delta-tui.visit-context-lines", "true"],
    );
    let path = ctx.dir.join(".git/config");
    let before = fs::read_to_string(&path).unwrap();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("T<down><enter>"));

    assert_eq!(app.state.theme.as_deref(), Some("theme-a"));
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn a_command_typed_is_not_saved_to_the_history() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    let mut app = ctx.init_app_as_pager_of(&["git", "diff"]);

    ctx.update(&mut app, keys(": HEAD<enter>"));

    assert!(!ctx.dir.join(".git/gitu").exists());
}

/// `read-only` in git config is a boolean, or `auto`.
#[test]
fn read_only_is_read_from_git_config() {
    let dir = temp_dir::TempDir::new().unwrap();
    let path = dir.path().join("gitconfig");
    for (value, expected) in [
        ("true", ReadOnly::On),
        ("off", ReadOnly::Off),
        ("auto", ReadOnly::Auto),
    ] {
        fs::write(&path, format!("[delta-tui]\n\tread-only = {value}\n")).unwrap();
        let git_config = git2::Config::open(&path).unwrap();
        let config = crate::config::init_config_from(
            PathBuf::from("/nonexistent/config.toml"),
            Some(&git_config),
        )
        .unwrap();
        assert_eq!(config.general.read_only, expected, "{value}");
    }
}

/// Whether `notice` is in the last column, under the commands on the item
/// under the cursor, which is headed `heading`, wrapped to fit.
fn notice_under(buffer: &str, heading: &str, notice: &str) -> bool {
    let rows = buffer.lines().collect::<Vec<_>>();
    let Some((heading_row, column)) = rows
        .iter()
        .enumerate()
        .find_map(|(i, row)| Some((i, row.find(&format!("   {heading}"))? + 3)))
    else {
        return false;
    };
    let text = rows[heading_row + 1..]
        .iter()
        .filter_map(|row| row.get(column..))
        .map(|rest| rest.trim_end_matches('|').trim())
        .filter(|rest| !rest.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    text.contains(notice)
}

/// The help menu says why commands are missing.
#[test]
fn the_help_menu_says_it_is_read_only() {
    let mut ctx = setup_clone!();
    read_only(&mut ctx);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("?"));

    let buffer = ctx.redact_buffer();
    assert!(
        notice_under(
            &buffer,
            "Section",
            "Read-only: commands that change the repository are hidden"
        ),
        "{buffer}"
    );
}

#[test]
fn the_help_menu_says_a_jj_repository_is_read_only() {
    let mut ctx = setup_clone!();
    colocate_jj(&ctx);
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("?"));

    let buffer = ctx.redact_buffer();
    assert!(
        notice_under(
            &buffer,
            "Section",
            "Read-only (.jj found): commands that change the repository are hidden"
        ),
        "{buffer}"
    );
}
