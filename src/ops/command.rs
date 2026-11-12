//! Shell commands the user defines in `general.commands` and binds to keys.

use crate::{
    Res,
    app::App,
    item_data::{ItemData, Ref},
    term::Term,
};
use std::process::Command;

/// Run the command `name`, with what it asks for filled in from the item under
/// the cursor, as a fetch is run: in the repository's root, while gitu stays
/// usable. Its output is not shown, unless with `--debug`; one that fails says
/// so. A command asking for what is not under the cursor is not run.
pub(crate) fn run(app: &mut App, term: &mut Term, name: &str) -> Res<()> {
    let Some(template) = app.state.config.general.commands.get(name) else {
        return Ok(());
    };
    let target = &app.screen().get_selected_item().data;
    let command = match filled_in(template, target) {
        Ok(command) => command,
        Err(placeholder) => {
            app.display_error(format!(
                "{name}: nothing under the cursor for {placeholder}"
            ));
            return Ok(());
        }
    };

    let mut cmd = Command::new("sh");
    cmd.args(["-c", &command]);
    app.run_cmd_async(term, &[], cmd)
}

/// `template` with each placeholder replaced by its value for `target`, quoted
/// for the shell, or the first placeholder that has none.
fn filled_in(template: &str, target: &ItemData) -> Result<String, &'static str> {
    let mut command = template.to_owned();
    for (placeholder, value) in [
        ("{commit}", commit_of(target)),
        ("{file}", file_of(target)),
        ("{branch}", branch_of(target)),
    ] {
        if !command.contains(placeholder) {
            continue;
        }
        let value = value.ok_or(placeholder)?;
        command = command.replace(placeholder, &shell_words::quote(&value));
    }
    Ok(command)
}

fn commit_of(target: &ItemData) -> Option<String> {
    match target {
        ItemData::Commit { oid, .. } => Some(oid.clone()),
        ItemData::BlameHeader { commit_hash, .. } | ItemData::BlameCodeLine { commit_hash, .. } => {
            Some(commit_hash.clone())
        }
        ItemData::Delta { commit, .. } => commit.clone(),
        ItemData::Hunk { diff, .. } | ItemData::HunkLine { diff, .. } => diff.commit.clone(),
        _ => None,
    }
}

/// The file a row is of. A deleted file is named as it was.
fn file_of(target: &ItemData) -> Option<String> {
    let (diff, file_i) = match target {
        ItemData::Untracked(path) => return Some(path.to_string_lossy().into_owned()),
        ItemData::Delta { diff, file_i, .. }
        | ItemData::Hunk { diff, file_i, .. }
        | ItemData::HunkLine { diff, file_i, .. } => (diff, *file_i),
        _ => return None,
    };
    let header = &diff.file_diffs[file_i].header;
    let path = match header.status {
        crate::gitu_diff::Status::Deleted => &header.old_file,
        _ => &header.new_file,
    };
    Some(path.fmt(&diff.text).into_owned())
}

/// The branch a row is, or the first one at the commit it is.
fn branch_of(target: &ItemData) -> Option<String> {
    match target {
        ItemData::Reference {
            kind: Ref::Head(branch),
            ..
        } => Some(branch.clone()),
        ItemData::Commit {
            associated_references,
            ..
        } => associated_references
            .iter()
            .find_map(|reference| match reference {
                Ref::Head(branch) => Some(branch.clone()),
                _ => None,
            }),
        _ => None,
    }
}
