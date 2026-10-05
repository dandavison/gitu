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

fn with_show_renderer(ctx: &mut TestContext, command: &[&str]) {
    let config = ctx.config();
    config.general.show_renderer.command = command.iter().map(|arg| arg.to_string()).collect();
}

/// A command of the user's own draws the header, so it can match their log
/// format and show the whole message; its first row heads the commit in place
/// of gitu's. gitu appends the commit to show.
#[test]
fn the_show_renderer_draws_the_commit_header() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    let head = run(&ctx.dir, &["git", "rev-parse", "HEAD"])
        .trim()
        .to_string();
    with_show_renderer(
        &mut ctx,
        &["sh", "-c", r#"printf 'shown %s\n' "$1""#, "gitu"],
    );

    show_head(&mut ctx);

    let buffer = ctx.redact_buffer();
    assert!(
        buffer.contains(&format!("shown {}", &head[..40])),
        "{buffer}"
    );
    assert!(!buffer.contains("Author:"), "{buffer}");
    assert!(!buffer.contains(&format!("commit {head}")), "{buffer}");
}

#[test]
fn a_failing_show_renderer_leaves_the_built_in_header() {
    let mut ctx = setup_clone!();
    commit(&ctx.dir, "firstfile", "testing\n");
    with_show_renderer(&mut ctx, &["false"]);

    show_head(&mut ctx);

    let buffer = ctx.redact_buffer();
    assert!(buffer.contains("Author:"), "{buffer}");
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
