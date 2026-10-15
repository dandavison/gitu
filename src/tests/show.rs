//! The commit view: a commit's header over its diff.

use super::pager::draws_lines_of;
use super::*;
use crate::{
    app::App,
    cli::{Args, Commands},
};
use std::time::{Duration, Instant};

fn show_head(ctx: &mut TestContext) -> App {
    let args = Args {
        command: Some(Commands::Show {
            reference: "HEAD".into(),
        }),
        ..Default::default()
    };
    ctx.init_app_with_args(ctx.dir.clone(), args)
}

/// The commit view is drawn once the renderer has drawn its first screen, and
/// the rest of the diff once it has drawn that.
#[test]
fn the_commit_view_is_drawn_before_the_renderer_finishes() {
    let mut ctx = setup_clone!();
    let lines = |prefix: &str| {
        (1..=60)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    commit(&ctx.dir, "a.rs", &lines("old a"));
    commit(&ctx.dir, "b.rs", "old b\n");
    fs::write(ctx.dir.join("a.rs"), lines("new a")).unwrap();
    fs::write(ctx.dir.join("b.rs"), "new b\n").unwrap();
    run(&ctx.dir, &["git", "commit", "-am", "change both"]);
    ctx.config().general.diff_renderer.command = super::pager::pausing_renderer();

    let started = Instant::now();
    let args = Args {
        command: Some(Commands::Show {
            reference: "HEAD".into(),
        }),
        ..Default::default()
    };
    let mut app = ctx.init_app_streaming_with_args(args);

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

/// The command a view is the output of, as `:` offers it for editing.
fn git_command_of(app: &App) -> Option<String> {
    app.screen()
        .git_command
        .as_ref()
        .map(|git| git.borrow().line(None))
}

/// A commit opened from the log is the view `git show` gives on the command
/// line, so the two behave the same.
#[test]
fn a_commit_opened_from_the_log_is_the_output_of_git_show() {
    let mut ctx = with_renderer(setup_clone!());
    commit(&ctx.dir, "firstfile", "testing\n");
    let head = run(&ctx.dir, &["git", "rev-parse", "HEAD"])
        .trim()
        .to_string();
    let mut app = ctx.init_app();

    ctx.update(&mut app, keys("ll<enter>"));

    assert_eq!(git_command_of(&app), Some(format!("git show {head}")));
}

#[test]
fn gitu_show_is_the_output_of_git_show() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let app = show_head(&mut ctx);

    assert_eq!(git_command_of(&app), Some("git show HEAD".into()));
}

/// Folding everything folds the files; the commit's header is not one of them.
#[test]
fn folding_all_leaves_the_commit_header_in_view() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let mut app = show_head(&mut ctx);

    ctx.update(&mut app, keys("<backtab>"));

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("Author:"), "{buffer}");
}

/// The arguments the user gives for the commits gitu shows, such as a pretty
/// format of their own.
#[test]
fn show_args_are_given_to_git_show() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    ctx.config().general.show_args = vec!["--format=shown %s".into()];

    show_head(&mut ctx);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("shown add firstfile"), "{buffer}");
}
