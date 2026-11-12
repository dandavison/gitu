use super::*;

fn prompt_after(ctx: TestContext, input: &str) -> String {
    let mut ctx = ctx;
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(input));
    ctx.prompt_line()
}

/// The row under the separator that a picker draws its search on.
fn picker_line(ctx: TestContext, input: &str) -> String {
    let mut ctx = ctx;
    let mut app = ctx.init_app();
    ctx.update(&mut app, keys(input));
    let buffer = ctx.redact_buffer();
    let mut lines = buffer.lines().skip_while(|line| !line.starts_with('─'));
    lines.next();
    lines
        .next()
        .unwrap()
        .trim_end_matches('|')
        .trim_end()
        .to_owned()
}

#[test]
fn text_prompt_is_its_label_and_a_colon() {
    assert_eq!(prompt_after(setup_clone!(), "zz"), "Stash message: |");
}

#[test]
fn text_prompt_gives_its_default_in_parentheses() {
    assert_eq!(
        prompt_after(setup_clone!(), "l-n-n"),
        "Limit number of commits (256): |"
    );
}

#[test]
fn text_prompt_label_has_one_colon() {
    assert_eq!(
        prompt_after(setup_clone!(), "bc"),
        "Create and checkout branch: |"
    );
}

#[test]
fn confirm_ends_in_y_n() {
    let ctx = setup_clone!();
    run(&ctx.dir, &["git", "branch", "asd"]);
    assert_eq!(prompt_after(ctx, "YjK"), "Really discard? [y/n] |");
}

#[test]
fn confirm_asks_a_question() {
    assert_eq!(
        prompt_after(setup_clone!(), "MKorigin<enter>"),
        "Really delete remote? [y/n] |"
    );
}

#[test]
fn picker_label_has_a_colon() {
    assert_eq!(picker_line(setup_clone!(), "bb"), " 2/2  Checkout: █");
}
