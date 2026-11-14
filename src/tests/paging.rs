use crate::cli::{Args, Commands};
use crate::pages;

/// As delta does, gitu reads a patch when one is piped to it, as git pipes one
/// to its pager.
#[test]
fn piped_input_is_paged() {
    assert!(pages(&Args::default(), false));
}

#[test]
fn input_from_the_terminal_opens_the_status_view() {
    assert!(!pages(&Args::default(), true));
}

/// git runs its sequence editor with whatever stdin it was given, which need
/// not be the terminal.
#[test]
fn a_subcommand_runs_whatever_its_input() {
    let args = Args {
        command: Some(Commands::Show {
            reference: "HEAD".to_owned(),
        }),
        ..Default::default()
    };
    assert!(!pages(&args, false));
}
