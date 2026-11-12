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

/// A command that fails says how, with what it wrote to stderr, and shows the
/// command as it was written, filled in, to be pasted into a shell: not the
/// `sh -c` that runs it.
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
    assert!(
        buffer.contains(&format!("exit code 3: echo {head} oops >&2; exit 3")),
        "{buffer}"
    );
    assert!(buffer.contains(&format!("  {head} oops")), "{buffer}");
    assert!(!buffer.contains("sh -c"), "{buffer}");
}
