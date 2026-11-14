//! Shell commands and views the user defines in `general.commands` and
//! `general.views` and binds to keys.

use crate::{
    Res,
    app::App,
    calling_process::GitCommand,
    error::Error,
    item_data::{ItemData, Ref},
    screen,
    term::Term,
};
use std::{rc::Rc, sync::Arc};

/// Run the command `name`, with what it asks for filled in from the item under
/// the cursor, as a fetch is run: in the repository's root, while gitu stays
/// usable, with the width and features gitu draws with in its environment. Its
/// output is not shown, unless with `--debug`; one that fails says so. A
/// command asking for what is not under the cursor is not run.
pub(crate) fn run(app: &mut App, term: &mut Term, name: &str) -> Res<()> {
    let Some(template) = app.state.config.general.commands.get(name) else {
        return Ok(());
    };
    let mut template = template.clone();
    if template.contains(COMMAND) {
        let Some((line, _)) = shown_command(app) else {
            app.display_error(no_command(name));
            return Ok(());
        };
        // Already as a shell takes it.
        template = template.replace(COMMAND, &line);
    }
    let target = &app.screen().get_selected_item().data;
    let command = match filled_in(&template, target, |value| {
        shell_words::quote(value).into_owned()
    }) {
        Ok(command) => command,
        Err(placeholder) => {
            app.display_error(nothing_for(app, name, placeholder));
            return Ok(());
        }
    };

    // A delta it runs draws as gitu draws the screen.
    let params = app.render_params(term.size().map_err(Error::Term)?);
    let env = crate::diff_renderer::drawing_env(&params);
    app.run_shell_async(term, &command, env)
}

/// Open the view `name` on top of the screen in view, with what it asks for
/// filled in from the item under the cursor. A view asking for what is not
/// under the cursor, or that is not a `git diff`, `git show` or `git log`, is
/// not opened.
pub(crate) fn open_view(app: &mut App, term: &mut Term, name: &str) -> Res<()> {
    let Some(template) = app.state.config.general.views.get(name) else {
        return Ok(());
    };
    let Ok(words) = shell_words::split(template) else {
        app.display_error(format!("{name}: {}", Error::EditedCommandQuotes));
        return Ok(());
    };
    // The command shown is several words, so it is a word of its own here, and
    // stands for those words.
    if words
        .iter()
        .any(|word| word != COMMAND && word.contains(COMMAND))
    {
        app.display_error(format!("{name}: {COMMAND} must be a word of its own"));
        return Ok(());
    }
    let mut prefix = String::new();
    let mut spliced = Vec::new();
    for word in words {
        if word != COMMAND {
            spliced.push(word);
            continue;
        }
        let Some((line, shown_prefix)) = shown_command(app) else {
            app.display_error(no_command(name));
            return Ok(());
        };
        // Its paths are relative to where it was run.
        prefix = shown_prefix;
        spliced.extend(shell_words::split(&line).map_err(|_| Error::EditedCommandQuotes)?);
    }

    let target = &app.screen().get_selected_item().data;
    // Word by word, so that a value with spaces stays one word.
    let argv: Result<Vec<String>, _> = spliced
        .iter()
        .map(|word| filled_in(word, target, |value| value.to_owned()))
        .collect();
    let argv = match argv {
        Ok(argv) => argv,
        Err(placeholder) => {
            app.display_error(nothing_for(app, name, placeholder));
            return Ok(());
        }
    };
    if GitCommand::of(argv.clone()).is_none() {
        app.display_error(format!(
            "{name}: a view is a git diff, git show or git log command"
        ));
        return Ok(());
    }

    let params = app.render_params(term.size().map_err(Error::Term)?);
    let screen = screen::pager::live_in(
        Arc::clone(&app.state.config),
        Rc::clone(&app.state.repo),
        params,
        argv,
        prefix,
    )?;
    app.state.screens.push(screen);
    Ok(())
}

/// The placeholder for the git command the screen is showing.
const COMMAND: &str = "{command}";

/// The git command the screen in view is showing, as `:` shows it, and where it
/// was run (see [`GitCommand::prefix`]). `None` for a screen that is not a git
/// command's output, such as the status view.
fn shown_command(app: &App) -> Option<(String, String)> {
    let git = app.screen().git_command.as_ref()?.borrow();
    Some((
        git.line(app.state.context.as_deref()),
        git.prefix().to_owned(),
    ))
}

fn no_command(name: &str) -> String {
    format!("{name}: this screen shows no git command, for {COMMAND}")
}

/// What to say when the command or view `name` asks for `placeholder` and the
/// cursor is on nothing that has one.
fn nothing_for(app: &App, name: &str, placeholder: &str) -> String {
    let message = format!("{name}: nothing under the cursor for {placeholder}");
    match app.screen().unmarked_commits_hint() {
        Some(hint) if placeholder == "{commit}" => format!("{message}: {hint}"),
        _ => message,
    }
}

/// `template` with each placeholder replaced by `written` of its value for
/// `target`, or the first placeholder that has none.
fn filled_in(
    template: &str,
    target: &ItemData,
    written: impl Fn(&str) -> String,
) -> Result<String, &'static str> {
    let mut filled = template.to_owned();
    for (placeholder, value) in [
        ("{commit}", commit_of(target)),
        ("{file}", file_of(target)),
        ("{branch}", branch_of(target)),
    ] {
        if !filled.contains(placeholder) {
            continue;
        }
        let value = value.ok_or(placeholder)?;
        filled = filled.replace(placeholder, &written(&value));
    }
    Ok(filled)
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
