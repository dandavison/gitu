use super::{Action, OpTrait};
use crate::{
    Res,
    app::{App, State},
    config::Config,
    item_data::ItemData,
    items::{self, Item},
    menu::arg::Arg,
    picker::{PickerData, PickerItem, PickerState},
    term::Term,
};
use ratatui::text::{Line, Span};
use std::{
    ffi::{OsStr, OsString},
    process::Command,
    rc::Rc,
    sync::Arc,
};

const COMMIT_PICKER_LIMIT: usize = 256;

pub(crate) fn init_args() -> Vec<Arg> {
    vec![
        Arg::new_flag("--all", "Stage all modified and deleted files", false),
        Arg::new_flag("--allow-empty", "Allow empty commit", false),
        Arg::new_flag("--verbose", "Show diff of changes to be committed", false),
        Arg::new_flag("--no-verify", "Disable hooks", false),
        Arg::new_flag(
            "--reset-author",
            "Claim authorship and reset author date",
            false,
        ),
        // TODO -A Override the author (--author=)
        Arg::new_flag("--signoff", "Add Signed-off-by line", false),
        // TODO -C Reuse commit message (--reuse-message=)
    ]
}

pub(crate) struct Commit;
impl OpTrait for Commit {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let mut cmd = Command::new("git");
            cmd.args(["commit"]);
            cmd.args(app.state.pending_menu.as_ref().unwrap().args());
            app.run_cmd_interactive(term, cmd)?;
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Commit".into()
    }
}

pub(crate) struct CommitAmend;
impl OpTrait for CommitAmend {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let mut cmd = Command::new("git");
            cmd.args(["commit", "--amend"]);
            cmd.args(app.state.pending_menu.as_ref().unwrap().args());
            app.run_cmd_interactive(term, cmd)?;
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "amend".into()
    }
}

pub(crate) struct CommitExtend;
impl OpTrait for CommitExtend {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let mut cmd = Command::new("git");
            cmd.args(["commit", "--amend", "--no-edit"]);
            cmd.args(app.state.pending_menu.as_ref().unwrap().args());
            app.run_cmd_interactive(term, cmd)?;
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "extend".into()
    }
}

pub(crate) struct CommitFixup;
impl OpTrait for CommitFixup {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let Some(rev) = pick_fixup_commit(app, term, "Fixup")? else {
                return Ok(());
            };
            let args = app.state.pending_menu.as_ref().unwrap().args();
            app.run_cmd_interactive(term, commit_fixup_cmd(&args, &rev))
        }))
    }

    fn display(&self, _state: &State) -> String {
        "fixup".into()
    }
}

fn commit_fixup_cmd(args: &[OsString], rev: &OsStr) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["commit", "--fixup"]);
    cmd.arg(rev);
    cmd.args(args);
    cmd
}

pub(crate) struct CommitInstantFixup;
impl OpTrait for CommitInstantFixup {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let Some(rev) = pick_fixup_commit(app, term, "Instant fixup")? else {
                return Ok(());
            };
            let args = app.state.pending_menu.as_ref().unwrap().args();
            app.run_cmd(term, &[], commit_fixup_cmd(&args, &rev))?;
            app.run_cmd(term, &[], rebase_autosquash_cmd(&rev))
        }))
    }

    fn display(&self, _state: &State) -> String {
        "instant fixup".into()
    }
}

/// Show a log-view-styled commit picker and return the chosen commit's oid.
fn pick_fixup_commit(
    app: &mut App,
    term: &mut Term,
    prompt: &'static str,
) -> Res<Option<OsString>> {
    let config = Arc::clone(&app.state.config);
    let log = items::log(&app.state.repo, COMMIT_PICKER_LIMIT, None, None)?;
    let picker_items = commit_picker_items(&log, &config);

    let cursor = selected_commit_oid(app).and_then(|oid| {
        picker_items
            .iter()
            .position(|item| item.data == PickerData::Item(oid.clone()))
    });

    let mut picker = PickerState::new(prompt, picker_items, false);
    if let Some(cursor) = cursor {
        picker.set_cursor(cursor);
    }

    Ok(app
        .pick(term, picker)?
        .map(|data| OsString::from(data.display())))
}

fn commit_picker_items(log: &[Item], config: &Arc<Config>) -> Vec<PickerItem> {
    log.iter()
        .filter_map(|item| match &item.data {
            ItemData::Commit {
                oid,
                short_id,
                summary,
                ..
            } => {
                let display = format!("{short_id} {summary}");
                let line = into_owned_line(item.to_line(Arc::clone(config)));
                Some(PickerItem::with_line(
                    display,
                    PickerData::Item(oid.clone()),
                    line,
                ))
            }
            _ => None,
        })
        .collect()
}

fn into_owned_line(line: Line) -> Line<'static> {
    line.spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect::<Vec<_>>()
        .into()
}

fn selected_commit_oid(app: &App) -> Option<String> {
    match &app.screen().get_selected_item().data {
        ItemData::Commit { oid, .. } => Some(oid.clone()),
        _ => None,
    }
}

fn rebase_autosquash_cmd(rev: &OsStr) -> Command {
    let mut cmd = Command::new("git");
    cmd.args([
        "rebase",
        "-i",
        "-q",
        "--autostash",
        "--keep-empty",
        "--autosquash",
    ]);
    cmd.arg(parent(rev));
    cmd.env("GIT_SEQUENCE_EDITOR", ":");
    cmd
}

fn parent(reference: &OsStr) -> OsString {
    let mut parent = reference.to_os_string();
    parent.push("^");
    parent
}
