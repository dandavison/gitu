use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Default, Debug, Parser)]
#[command(name = "gitu")]
#[command(flatten_help = true)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Send keys on startup (eg: `gitu -k ll`).
    ///     It is possible to send:
    ///     - single char-keys: a, b, c, ...
    ///     - special keys: <backspace>, <enter>, <up>, <tab>, <delete>, <esc>, ...
    ///     - modifiers: <ctrl+a>, <ctrl+shift+alt+a>, <shift+delete>
    #[clap(short, long, verbatim_doc_comment)]
    pub keys: Option<String>,

    /// Print one frame and exit. Useful for debugging.
    #[clap(long, action)]
    pub print: bool,

    /// Read a patch on stdin and browse it, for use as git's pager:
    ///     GIT_PAGER='gitu --pager' git show
    #[clap(long, action, verbatim_doc_comment)]
    pub pager: bool,

    /// Enable logging to 'gitu.log'
    #[clap(long, action)]
    pub log: bool,

    #[clap(long, action)]
    /// Print version
    pub version: bool,

    /// Config file to use
    #[clap(short, long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Show {
        reference: String,
    },
    /// Interactively rebase onto <upstream>, editing the instruction list in Gitu.
    Rebase {
        upstream: String,
    },
    /// Edit a `git rebase -i` instruction list, for use as GIT_SEQUENCE_EDITOR:
    ///     GIT_SEQUENCE_EDITOR='gitu sequence-editor' git rebase -i <upstream>
    #[clap(verbatim_doc_comment)]
    SequenceEditor {
        file: PathBuf,
    },
    Blame {
        file: String,
        #[clap(short, long)]
        rev: Option<String>,
    },
}
