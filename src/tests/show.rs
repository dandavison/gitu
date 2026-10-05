//! The commit view: a commit's header over its diff.

use super::*;
use crate::{
    app::App,
    cli::{Args, Commands},
};

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
    config.general.show_renderer.enabled = true;
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
