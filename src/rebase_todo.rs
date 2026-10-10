//! An editable `git rebase -i` instruction list.
//!
//! gitu doesn't compose the todo itself. It comes from git, in one of two ways:
//!
//! - [`RebaseTodo::capture`]: gitu starts the rebase, so it asks git for the todo
//!   it would have opened in `$EDITOR` (honouring `--autosquash`,
//!   `--rebase-merges` and `rebase.instructionFormat`) and, once edited, runs the
//!   rebase for real with the result. Both halves use `GIT_SEQUENCE_EDITOR`:
//!   capturing copies the todo out and fails, which makes git abort without
//!   touching anything; applying copies the edited list in.
//! - [`RebaseTodo::read`]: git started the rebase and ran gitu *as* its sequence
//!   editor, handing over the file to edit in place.

use crate::{Res, error::Error};
use git2::Repository;
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Sequence editors, parameterised by environment (rather than by quoting a
/// path into the command string, which git runs through a shell). `$1` is the
/// todo path git appends.
const CAPTURE_EDITOR: &str = r#"sh -c 'cat -- "$1" > "$GITU_REBASE_TODO"; exit 1' --"#;
const APPLY_EDITOR: &str = r#"sh -c 'cat -- "$GITU_REBASE_TODO" > "$1"' --"#;
const TODO_VAR: &str = "GITU_REBASE_TODO";
const TODO_FILE: &str = "gitu-rebase-todo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TodoAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl TodoAction {
    pub(crate) fn keyword(&self) -> &'static str {
        match self {
            TodoAction::Pick => "pick",
            TodoAction::Reword => "reword",
            TodoAction::Edit => "edit",
            TodoAction::Squash => "squash",
            TodoAction::Fixup => "fixup",
            TodoAction::Drop => "drop",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        match word {
            "p" | "pick" => Some(TodoAction::Pick),
            "r" | "reword" => Some(TodoAction::Reword),
            "e" | "edit" => Some(TodoAction::Edit),
            "s" | "squash" => Some(TodoAction::Squash),
            "f" | "fixup" => Some(TodoAction::Fixup),
            "d" | "drop" => Some(TodoAction::Drop),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum TodoEntry {
    /// An entry acting on a commit, whose oid we resolve in full so the log
    /// renderer's rows can be matched to it.
    Commit { action: TodoAction, oid: String },
    /// Anything else git put in the list (`exec`, `break`, `label`, …), kept
    /// verbatim: it can be reordered or dropped but not re-marked.
    Other(String),
}

#[derive(Debug)]
pub(crate) struct RebaseTodo {
    /// Newest first, as the log view lists commits. git's instruction list runs
    /// the other way, so it is reversed on the way in and out.
    pub entries: Vec<TodoEntry>,
    source: Source,
}

/// Where the list came from, and so what starting it does.
#[derive(Debug)]
enum Source {
    /// gitu captured the list and runs the rebase itself.
    Captured {
        /// The rev being rebased onto, as given to `git rebase -i`.
        base: OsString,
        /// The rebase menu's arguments, replayed when the list is applied.
        args: Vec<OsString>,
        todo_file: PathBuf,
        workdir: PathBuf,
    },
    /// git is mid-rebase and gave gitu its file to edit in place.
    Editing(PathBuf),
}

impl RebaseTodo {
    /// Ask git for the todo it would open for `git rebase -i <args> <base>`.
    pub(crate) fn capture(repo: &Repository, base: &OsStr, args: &[OsString]) -> Res<Self> {
        let workdir = repo.workdir().ok_or(Error::NoRepoWorkdir)?.to_path_buf();
        let todo_file = repo.path().join(TODO_FILE);

        // The editor exits non-zero once it has the list, so git aborts the
        // rebase before doing anything (an --autostash is created and restored).
        let output = Command::new("git")
            .args(["rebase", "-i"])
            .args(args)
            .arg(base)
            .env("GIT_SEQUENCE_EDITOR", CAPTURE_EDITOR)
            .env(TODO_VAR, &todo_file)
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(Error::SpawnCmd)?;

        // No list means git bailed before reaching the editor; it says why.
        let Ok(text) = fs::read_to_string(&todo_file) else {
            return Err(Error::CmdBadExit(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
                output.status.code(),
            ));
        };
        let _ = fs::remove_file(&todo_file);

        Ok(Self {
            entries: read_entries(repo, &text),
            source: Source::Captured {
                base: base.to_os_string(),
                args: args.to_vec(),
                todo_file,
                workdir,
            },
        })
    }

    /// Read the list git handed us as its sequence editor.
    pub(crate) fn read(repo: &Repository, path: &Path) -> Res<Self> {
        let text = fs::read_to_string(path).map_err(Error::ReadRebaseTodo)?;

        Ok(Self {
            entries: read_entries(repo, &text),
            source: Source::Editing(path.to_path_buf()),
        })
    }

    /// The revs naming this list's commits, for rendering them: exactly the
    /// entries', in the order held, whatever the rebase is doing to them.
    pub(crate) fn revs(&self) -> Vec<String> {
        let commits = self.entries.iter().filter_map(|entry| match entry {
            TodoEntry::Commit { oid, .. } => Some(oid.clone()),
            TodoEntry::Other(_) => None,
        });

        let mut revs: Vec<String> = commits.collect();
        if !revs.is_empty() {
            revs.insert(0, "--no-walk=unsorted".to_string());
        }
        revs
    }

    /// Hand the edited list back to git. In [`Source::Captured`] that means a
    /// command the caller runs; when editing git's own file there is nothing to
    /// run — writing it is the whole job.
    pub(crate) fn start(&self) -> Res<Option<Command>> {
        match &self.source {
            Source::Captured { todo_file, .. } => {
                fs::write(todo_file, self.text()).map_err(Error::WriteRebaseTodo)?;
                Ok(Some(self.apply_cmd(todo_file)))
            }
            Source::Editing(path) => {
                fs::write(path, self.text()).map_err(Error::WriteRebaseTodo)?;
                Ok(None)
            }
        }
    }

    /// The command that runs the rebase with this list in place of the todo git
    /// generates. The caller runs it interactively: `reword`/`squash` open an
    /// editor and `edit` stops the rebase.
    fn apply_cmd(&self, todo_file: &Path) -> Command {
        let Source::Captured {
            base,
            args,
            workdir,
            ..
        } = &self.source
        else {
            unreachable!("only a captured list is applied by running git")
        };

        let mut cmd = Command::new("git");
        cmd.args(["rebase", "-i"])
            .args(args)
            .arg(base)
            .env("GIT_SEQUENCE_EDITOR", APPLY_EDITOR)
            .env(TODO_VAR, todo_file)
            .current_dir(workdir);
        cmd
    }

    /// Remove the list we handed git, once the rebase has read it.
    pub(crate) fn discard_file(&self) {
        if let Source::Captured { todo_file, .. } = &self.source {
            let _ = fs::remove_file(todo_file);
        }
    }

    fn text(&self) -> String {
        self.entries
            .iter()
            .rev()
            .map(|entry| match entry {
                TodoEntry::Commit { action, oid } => format!("{} {oid}\n", action.keyword()),
                TodoEntry::Other(line) => format!("{line}\n"),
            })
            .collect()
    }

    /// Move the entry at `index` by `offset` places, returning where it landed.
    pub(crate) fn move_entry(&mut self, index: usize, offset: isize) -> usize {
        let to = index.saturating_add_signed(offset);
        if index >= self.entries.len() || to >= self.entries.len() {
            return index;
        }

        let entry = self.entries.remove(index);
        self.entries.insert(to, entry);
        to
    }

    pub(crate) fn set_action(&mut self, index: usize, action: TodoAction) {
        if let Some(TodoEntry::Commit {
            action: current, ..
        }) = self.entries.get_mut(index)
        {
            *current = action;
        }
    }
}

/// Read git's instruction list into display order, resolving each entry's
/// abbreviated oid so it can be matched against rendered log rows (and written
/// back unambiguously).
fn read_entries(repo: &Repository, text: &str) -> Vec<TodoEntry> {
    let mut entries = parse(repo, text);
    entries.reverse();
    entries
}

fn parse(repo: &Repository, text: &str) -> Vec<TodoEntry> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut words = line.split_whitespace();
            let action = words.next().and_then(TodoAction::parse);
            let oid = words
                .next()
                .and_then(|rev| repo.revparse_single(rev).ok())
                .map(|object| object.id().to_string());

            match (action, oid) {
                (Some(action), Some(oid)) => TodoEntry::Commit { action, oid },
                _ => TodoEntry::Other(line.to_string()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(oids: &[&str]) -> RebaseTodo {
        RebaseTodo {
            source: Source::Editing(PathBuf::new()),
            entries: oids
                .iter()
                .map(|oid| TodoEntry::Commit {
                    action: TodoAction::Pick,
                    oid: oid.to_string(),
                })
                .collect(),
        }
    }

    fn oids(todo: &RebaseTodo) -> Vec<&str> {
        todo.entries
            .iter()
            .map(|entry| match entry {
                TodoEntry::Commit { oid, .. } => oid.as_str(),
                TodoEntry::Other(line) => line.as_str(),
            })
            .collect()
    }

    #[test]
    fn move_entry_shifts_and_reports_where_it_landed() {
        let mut todo = todo(&["a", "b", "c"]);
        assert_eq!(todo.move_entry(0, 1), 1);
        assert_eq!(oids(&todo), ["b", "a", "c"]);
        assert_eq!(todo.move_entry(1, -1), 0);
        assert_eq!(oids(&todo), ["a", "b", "c"]);
    }

    #[test]
    fn move_entry_stops_at_the_ends() {
        let mut todo = todo(&["a", "b"]);
        assert_eq!(todo.move_entry(0, -1), 0);
        assert_eq!(todo.move_entry(1, 1), 1);
        assert_eq!(oids(&todo), ["a", "b"]);
    }

    #[test]
    fn text_is_one_instruction_per_entry_oldest_first() {
        // The entries are held newest first, as the log view shows them; git's
        // list runs the other way.
        let mut todo = todo(&["aaa", "bbb"]);
        todo.set_action(1, TodoAction::Fixup);
        todo.entries.push(TodoEntry::Other("exec make test".into()));

        assert_eq!(todo.text(), "exec make test\nfixup bbb\npick aaa\n");
    }

    #[test]
    fn set_action_leaves_non_commit_entries_alone() {
        let mut todo = todo(&[]);
        todo.entries.push(TodoEntry::Other("break".into()));
        todo.set_action(0, TodoAction::Drop);
        assert_eq!(todo.text(), "break\n");
    }
}
