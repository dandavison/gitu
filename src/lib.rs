pub mod app;
mod bindings;
mod calling_process;
pub mod cli;
mod cmd_log;
pub mod config;
mod diff_colorizer;
pub mod error;
mod file_watcher;
mod git;
pub mod gitu_diff;
mod highlight;
mod item_data;
mod items;
mod key_parser;
mod menu;
mod ops;
pub mod picker;
mod prompt;
mod rebase_todo;
mod screen;
mod syntax_parser;
pub mod term;
#[cfg(test)]
mod tests;
mod ui;

use bindings::Bindings;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventState, KeyModifiers};
use error::Error;
use git2::Repository;
use items::Item;
use ops::Action;
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use term::Term;

use crate::config::Config;
use crate::screen::pager::Paged;

pub const LOG_FILE_NAME: &str = "gitu.log";

//                                An overview of Gitu's ui and terminology:
//
//                Screen (src/screen/*)
//                  │
//                  ▼
//                 ┌──────────────────────────────────────────────────────────────────┐
//        Item───┬─► On branch master                                                 │
//        Item   └─► Your branch is up to date with 'origin/master'.                  │
//        ...      │                                                                  │
//                 │ Untracked files                                                  │
//                 │ src/tests/rebase.rs                                              │
//                 │                                                                  │
//                 │ Unstaged changes (4)                                             │
//                 │▌modified   src/keybinds.rs…                                      │
//                 │ modified   src/ops/mod.rs…                                       │
//                 │ modified   src/ops/rebase.rs…                                    │
//                 │ modified   src/tests/mod.rs…                                     │
//                 │                                                                  │
//                 │ Stashes                                                          │
//                 │ stash@0 On master: scroll                                        │
//                 │ stash@1 WIP on fix/run-cmd-error-on-bad-exit: 098d14a feat: prom…│
//                 │                                                                  │
//                 │ Recent commits                                                   │
// Ops (src/ops/*) ├──────────────────────────────────────────────────────────────────┤
//       │         │Help                        Submenu      modified   src/keybinds.r│
//       └─────┬───►g Refresh                   h Help       ret Show                 │
//             └───►tab Toggle section          b Branch     K Discard                │
//                 │k p ↑ Move up               c Commit     s Stage                  │
//                 │j n ↓ Move down             f Fetch      u Unstage                │
// Submenu ───────►│C-k C-p C-↑ Move up line    l Log                                 │
//                 │C-j C-n C-↓ Move down line  F Pull                                │
//                 │C-u Half page up            P Push                                │
//                 │C-d Half page down          r Rebase                              │
//                 │y Show refs                 X Reset                               │
//                 │                            z Stash                               │
//                 └──────────────────────────────────────────────────────────────────┘

pub type Res<T> = Result<T, Error>;

/// Run gitu, returning the status to exit with.
pub fn run(config: Arc<Config>, args: &cli::Args, term: &mut Term) -> Res<i32> {
    // Before anything slower, and before reading the input: the git process on
    // the other end of a pipe exits as soon as its output fits in the pipe, and
    // cannot be found once it has. Reading to EOF is always too late — EOF is
    // git closing the pipe.
    let git_argv = args.pager.then(calling_process::paging_for).flatten();
    log::debug!("Paging the output of {git_argv:?}");

    let dir = find_git_dir()?;
    let repo = open_repo(&dir)?;

    let piped = args.pager.then(|| read_piped_input(git_argv)).transpose()?;

    let mut app = app::App::create(
        Rc::new(repo),
        term.size().map_err(Error::Term)?,
        args,
        config,
        true,
        piped,
    )?;

    if let Some(keys_string) = &args.keys {
        let ("", keys) = key_parser::parse_keys(keys_string).expect("Couldn't parse keys") else {
            panic!("Couldn't parse keys");
        };

        for event in keys_to_events(&keys) {
            app.handle_event(term, event)?;
        }
    }

    app.redraw_now(term)?;

    if args.print {
        return Ok(app.state.exit_code);
    }

    app.run(term, Duration::from_millis(100))?;

    Ok(app.state.exit_code)
}

/// What git piped to us as its pager, alongside the command already found to
/// have produced it. Reading the terminal instead would wait for input that is
/// never coming, so that is refused outright.
fn read_piped_input(git_argv: Option<Vec<String>>) -> Res<Paged> {
    if io::stdin().is_terminal() {
        return Err(Error::PagerWithoutInput);
    }

    Ok(Paged {
        text: io::read_to_string(io::stdin()).map_err(Error::ReadPipedInput)?,
        git_argv,
    })
}

fn open_repo(dir: &Path) -> Res<Repository> {
    log::debug!("Opening repo");
    let repo = open_repo_from_env()?;
    repo.set_workdir(dir, false).map_err(Error::OpenRepo)?;
    Ok(repo)
}

fn find_git_dir() -> Res<PathBuf> {
    log::debug!("Finding git dir");
    let dir = PathBuf::from(
        String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .map_err(Error::FindGitDir)?
                .stdout,
        )
        .map_err(Error::GitDirUtf8)?
        .trim_end(),
    );
    Ok(dir)
}

fn open_repo_from_env() -> Res<Repository> {
    match Repository::open_from_env() {
        Ok(repo) => Ok(repo),
        Err(err) => Err(Error::OpenRepo(err)),
    }
}

fn keys_to_events(keys: &[(KeyModifiers, KeyCode)]) -> Vec<Event> {
    keys.iter()
        .map(|(mods, key)| {
            Event::Key(KeyEvent {
                code: *key,
                modifiers: *mods,
                kind: event::KeyEventKind::Press,
                state: KeyEventState::NONE,
            })
        })
        .collect::<Vec<_>>()
}
