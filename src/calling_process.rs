//! The git command whose output gitu is paging.
//!
//! Set as git's pager, gitu is handed a patch and told nothing about the
//! question it answers — not the subcommand, not the revs, not the options. The
//! command is in the process tree, though: git spawns its pager as a child, and
//! a shell pipeline puts the two side by side. Finding it there is what lets
//! gitu ask git the same question again — for more of the file around each
//! change — and know what the patch is a diff *of*, which is what decides
//! whether staging part of it means anything.

use crate::git::diff::DiffType;
use std::path::Path;
use std::process::Command;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// The subcommands whose output gitu structures.
const SUBCOMMANDS: [&str; 3] = ["diff", "show", "log"];

/// How far above gitu to look for the git process: git may spawn its pager
/// through a shell, and that shell through another.
const ANCESTORS: usize = 3;

/// A git invocation gitu can put again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitCommand {
    argv: Vec<String>,
    /// Where the subcommand sits in `argv`. What precedes it is git's own
    /// (`-c k=v`, `--git-dir=…`) and has to stay in front of it.
    subcommand: usize,
}

impl GitCommand {
    /// The invocation `argv` is, if it is one whose output gitu structures.
    pub(crate) fn of(argv: Vec<String>) -> Option<Self> {
        let program = Path::new(argv.first()?).file_stem()?.to_str()?;
        if !program.eq_ignore_ascii_case("git") {
            return None;
        }

        let subcommand = argv
            .iter()
            .position(|arg| SUBCOMMANDS.contains(&arg.as_str()))?;
        Some(Self { argv, subcommand })
    }

    /// What the patch is a diff of, which is what decides whether staging or
    /// unstaging part of it means anything (see [`DiffType`]). Only a `git diff`
    /// of the working tree or of the index is either of those; naming a rev
    /// makes it a comparison of two trees, as `git show` and `git log -p`
    /// always are.
    pub(crate) fn diff_type(&self) -> DiffType {
        if self.subcommand() != "diff" {
            return DiffType::TreeToTree;
        }

        let args: Vec<&str> = self.arguments().take_while(|arg| *arg != "--").collect();
        if args.contains(&"--cached") || args.contains(&"--staged") {
            DiffType::IndexToTree
        } else if args.iter().any(|arg| !arg.starts_with('-')) {
            // The value of a separated option (`-U 8`) reads as a rev here, so
            // such a diff is taken for a comparison of two trees. Nothing reads
            // the other way round, so the mistake only ever narrows the ops.
            DiffType::TreeToTree
        } else {
            DiffType::WorkdirToIndex
        }
    }

    /// The command again, asking for `context` around each change (`-U8`,
    /// `-W`) in place of whatever it originally asked for. `None` puts the
    /// question exactly as git put it.
    pub(crate) fn command(&self, context: Option<&str>) -> Command {
        let mut command = Command::new(&self.argv[0]);
        command.args(&self.argv[1..=self.subcommand]);
        // gitu parses what comes back, so an external differ must not stand in
        // for the patch, and git's colours have to be off however the user has
        // configured them.
        command.args(["--no-ext-diff", "--no-color"]);
        command.args(context);
        match context {
            Some(_) => command.args(without_context(self.arguments())),
            None => command.args(self.arguments()),
        };
        command
    }

    fn subcommand(&self) -> &str {
        &self.argv[self.subcommand]
    }

    /// What was asked of the subcommand.
    fn arguments(&self) -> impl Iterator<Item = &str> + Clone {
        self.argv[self.subcommand + 1..].iter().map(String::as_str)
    }
}

/// `args` with whatever they said about context dropped, so that what gitu asks
/// for is what git is asked for.
fn without_context<'a>(args: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut kept = Vec::new();
    let mut value_follows = false;

    for arg in args {
        if std::mem::take(&mut value_follows) {
            continue;
        }
        match arg {
            "-U" | "--unified" => value_follows = true,
            "-W" | "--function-context" | "--no-function-context" => {}
            _ if arg.starts_with("-U") || arg.starts_with("--unified=") => {}
            _ => kept.push(arg),
        }
    }

    kept
}

/// The argv of the git command gitu is paging the output of, looked for in the
/// two places it can be: above gitu, which is where git puts its pager, and
/// beside it, which is where a shell pipeline puts it.
pub(crate) fn paging_for() -> Option<Vec<String>> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );

    let me = Pid::from_u32(std::process::id());

    let mut descendant = me;
    for _ in 0..ANCESTORS {
        let Some(ancestor) = system.process(descendant).and_then(Process::parent) else {
            break;
        };
        if let Some(argv) = git_argv(&system, ancestor) {
            return Some(argv);
        }
        descendant = ancestor;
    }

    // `git diff | gitu --pager`: the shell created both, so git is the sibling
    // of gitu rather than its ancestor. Two of them and there is no telling
    // which produced the input.
    let shell = system.process(me)?.parent()?;
    let mut siblings = system
        .processes()
        .iter()
        .filter(|(pid, process)| **pid != me && process.parent() == Some(shell))
        .filter_map(|(pid, _)| git_argv(&system, *pid));

    siblings.next().filter(|_| siblings.next().is_none())
}

/// The argv of `pid`, if it is a git command gitu can put again.
fn git_argv(system: &System, pid: Pid) -> Option<Vec<String>> {
    let argv: Vec<String> = system
        .process(pid)?
        .cmd()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    GitCommand::of(argv).map(|git| git.argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(args: &[&str]) -> GitCommand {
        GitCommand::of(args.iter().map(|arg| (*arg).to_owned()).collect()).unwrap()
    }

    fn rerun(args: &[&str], context: Option<&str>) -> Vec<String> {
        let command = git(args).command(context);
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_git_command_is_recognised_behind_its_own_options() {
        assert_eq!(
            git(&["/usr/bin/git", "-c", "color.diff=always", "diff"]).subcommand(),
            "diff"
        );
        assert_eq!(git(&["git.exe", "show", "HEAD"]).subcommand(), "show");
        assert_eq!(GitCommand::of(vec!["ls".to_owned()]), None);
        assert_eq!(GitCommand::of(vec!["git".to_owned()]), None);
        assert_eq!(
            GitCommand::of(["git", "status"].map(String::from).to_vec()),
            None
        );
    }

    /// Whether a patch is of the working tree, of the index, or of two trees is
    /// what the command says it is.
    #[test]
    fn the_command_says_what_the_patch_is_a_diff_of() {
        assert_eq!(git(&["git", "diff"]).diff_type(), DiffType::WorkdirToIndex);
        assert_eq!(
            git(&["git", "diff", "-U8", "--", "src/lib.rs"]).diff_type(),
            DiffType::WorkdirToIndex
        );
        assert_eq!(
            git(&["git", "diff", "--cached"]).diff_type(),
            DiffType::IndexToTree
        );
        assert_eq!(
            git(&["git", "diff", "main"]).diff_type(),
            DiffType::TreeToTree
        );
        assert_eq!(
            git(&["git", "show", "HEAD"]).diff_type(),
            DiffType::TreeToTree
        );
    }

    #[test]
    fn asking_again_keeps_the_question_and_replaces_the_context() {
        assert_eq!(
            rerun(&["git", "-c", "color.ui=always", "diff", "main"], None),
            [
                "git",
                "-c",
                "color.ui=always",
                "diff",
                "--no-ext-diff",
                "--no-color",
                "main"
            ]
        );
        assert_eq!(
            rerun(&["git", "diff", "-U10", "main", "--", "src"], Some("-U2")),
            [
                "git",
                "diff",
                "--no-ext-diff",
                "--no-color",
                "-U2",
                "main",
                "--",
                "src"
            ]
        );
        assert_eq!(
            rerun(&["git", "diff", "--unified", "10"], Some("-W")),
            ["git", "diff", "--no-ext-diff", "--no-color", "-W"]
        );
        assert_eq!(
            rerun(&["git", "diff", "--function-context"], Some("-U3")),
            ["git", "diff", "--no-ext-diff", "--no-color", "-U3"]
        );
    }
}
