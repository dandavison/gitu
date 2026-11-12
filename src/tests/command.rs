//! Shell commands the user defines in config and binds to keys.

use super::*;

/// `ctx` with `command` defined as `name`, bound to `key`.
fn with_command(mut ctx: TestContext, name: &str, command: &str, key: char) -> TestContext {
    let config = ctx.config();
    config
        .general
        .commands
        .insert(name.to_owned(), command.to_owned());
    config
        .general
        .diff_renderer
        .feature_keys
        .insert(name.to_owned(), key);
    ctx
}

/// `{commit}` is the commit under the cursor.
#[test]
fn a_command_key_runs_the_command_on_the_commit_under_the_cursor() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "a", "");
    let mut ctx = with_command(ctx, "record", "echo {commit} > recorded", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("llo"));

    let head = run(&ctx.dir, &["git", "rev-parse", "HEAD"]);
    assert_eq!(
        fs::read_to_string(ctx.dir.join("recorded")).unwrap(),
        head,
        "{}",
        ctx.redact_buffer()
    );
}

/// `{file}` is the file under the cursor, quoted for the shell.
#[test]
fn a_command_key_runs_the_command_on_the_file_under_the_cursor() {
    let ctx = setup_clone!();
    fs::write(ctx.dir.join("a file"), "").unwrap();
    let mut ctx = with_command(ctx, "record", "echo {file} > recorded", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("/a file<enter>o"));

    assert_eq!(
        fs::read_to_string(ctx.dir.join("recorded")).unwrap(),
        "a file\n",
        "{}",
        ctx.redact_buffer()
    );
}

/// A command asking for what is not under the cursor is not run, and says why.
#[test]
fn a_command_needing_a_commit_is_not_run_off_one() {
    let ctx = setup_clone!();
    let mut ctx = with_command(ctx, "record", "echo {commit} > recorded", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("o"));

    assert!(!ctx.dir.join("recorded").exists());
    assert!(
        ctx.redact_buffer().contains("{commit}"),
        "{}",
        ctx.redact_buffer()
    );
}

/// The list of features lists the commands too, where their keys are set.
#[test]
fn the_feature_list_lists_commands() {
    let ctx = setup_clone!();
    let mut ctx = with_command(ctx, "record", "echo {commit} > recorded", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("H"));

    assert!(
        ctx.redact_buffer().contains("o record"),
        "{}",
        ctx.redact_buffer()
    );
}

/// A command that fails is shown as it was written, filled in, on a line of its
/// own, to be pasted into a shell (not the `sh -c` that runs it), then how it
/// exited and what it wrote to stderr.
#[test]
fn a_failing_command_shows_its_stderr_and_the_command_as_written() {
    let ctx = setup_clone!();
    commit(&ctx.dir, "a", "");
    let mut ctx = with_command(ctx, "fail", "echo {commit} oops >&2; exit 3", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("llo"));

    let head = run(&ctx.dir, &["git", "rev-parse", "HEAD"]);
    let head = head.trim();
    let buffer = ctx.redact_buffer();
    let rows: Vec<&str> = buffer
        .lines()
        .skip_while(|row| !row.starts_with('!'))
        .take(4)
        .map(str::trim_end)
        .map(|row| row.trim_end_matches('|').trim_end())
        .collect();
    assert_eq!(
        rows,
        [
            format!("! echo {head} oops >&2; exit 3"),
            "  exit code: 3".to_owned(),
            "  stderr:".to_owned(),
            format!("  {head} oops"),
        ],
        "{buffer}"
    );
}

/// `{command}` is the git command the screen is showing, as `:` shows it.
#[test]
fn a_command_key_runs_the_command_on_the_git_command_shown() {
    let ctx = setup_clone!();
    let mut ctx = with_command(ctx, "record", "echo {command} > recorded", 'o');
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("llo"));

    let recorded = fs::read_to_string(ctx.dir.join("recorded")).unwrap_or_default();
    assert!(recorded.starts_with("git log"), "{recorded:?}");
}

/// A command is given the width and the features gitu draws with, as delta
/// takes them, so that a delta it runs draws as the screen is drawn.
#[test]
fn a_command_gets_the_width_and_features_gitu_draws_with() {
    let ctx = setup_clone!();
    let mut ctx = with_command(
        ctx,
        "record",
        r#"printf '%s %s' "$COLUMNS" "$DELTA_FEATURES" > recorded"#,
        'o',
    );
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("So"));

    let recorded = fs::read_to_string(ctx.dir.join("recorded")).unwrap_or_default();
    assert!(recorded.starts_with("78 "), "{recorded:?}");
    assert!(recorded.contains("side-by-side"), "{recorded:?}");
}

/// A command may write more than a pipe holds. Were its output read only once
/// it exits, it would wait forever for room to write; here the command stops
/// its writer after 5s, and does not record that it finished.
#[test]
fn a_command_writing_much_output_finishes() {
    let ctx = setup_clone!();
    let mut ctx = with_command(
        ctx,
        "write",
        "yes | head -c 1000000 & w=$!; \
         (sleep 5; kill $w) >/dev/null 2>&1 & \
         wait $w && touch finished",
        'o',
    );
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("o"));

    assert!(ctx.dir.join("finished").exists(), "{}", ctx.redact_buffer());
}
