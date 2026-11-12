use super::OpTrait;
use crate::{
    Action,
    app::State,
    error::Error,
    item_data::{ItemData, Ref},
    items::RenderParams,
    screen::{self, Screen},
};
use core::str;
use std::{path::Path, process::Command, rc::Rc, sync::Arc};

pub(crate) struct Show;
impl OpTrait for Show {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        show_screen(target).or_else(|| show_in_editor(target))
    }
    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, _state: &State) -> String {
        "Show".into()
    }
}

/// Show without leaving gitu: a commit, tag or stash opens on its own screen,
/// and a folded file or hunk unfolds. Anything else, such as a file already
/// unfolded, is left as it is.
pub(crate) struct ShowOrUnfold;
impl OpTrait for ShowOrUnfold {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        if let Some(action) = show_screen(target) {
            return Some(action);
        }
        if !target.is_section() {
            return None;
        }
        Some(Rc::new(|app, _term| {
            let screen = app.screen_mut();
            if screen.is_collapsed(screen.get_selected_item()) {
                screen.toggle_section()?;
            }
            Ok(())
        }))
    }

    fn is_target_op(&self) -> bool {
        true
    }

    // The help lists this as Unfold on a section already unfolded, where it does
    // nothing, since which ops are listed is decided without the fold state.
    fn display(&self, state: &State) -> String {
        let item = state.screens.last().unwrap().get_selected_item();
        if show_screen(&item.data).is_some() {
            "Show".into()
        } else {
            "Unfold".into()
        }
    }
}

fn show_screen(target: &ItemData) -> Option<Action> {
    match target {
        ItemData::Commit { oid, .. }
        | ItemData::Reference {
            kind: Ref::Tag(oid),
            ..
        }
        | ItemData::Reference {
            kind: Ref::Head(oid),
            ..
        } => goto_show_screen(oid.clone(), None),
        ItemData::BlameHeader {
            commit_hash,
            file_path,
            line_num,
            ..
        } if !is_null_oid(commit_hash) => {
            goto_show_screen(commit_hash.clone(), Some((file_path.clone(), *line_num)))
        }
        ItemData::BlameCodeLine {
            commit_hash,
            file_path,
            orig_line_num,
            ..
        } if !is_null_oid(commit_hash) => goto_show_screen(
            commit_hash.clone(),
            Some((file_path.clone(), *orig_line_num)),
        ),
        ItemData::Stash { stash_ref, .. } => goto_show_stash_screen(stash_ref.clone()),
        _ => None,
    }
}

fn show_in_editor(target: &ItemData) -> Option<Action> {
    match target {
        ItemData::Untracked(u) => editor(u.as_path(), None),
        ItemData::Delta { diff, file_i, .. } => {
            let file_path = &diff.file_diffs[*file_i].header.new_file;
            let path: &str = &file_path.fmt(&diff.text);
            editor(Path::new(path), None)
        }
        ItemData::Hunk {
            diff,
            file_i,
            hunk_i,
        } => {
            let file_path = &diff.file_diffs[*file_i].header.new_file;
            let path: &str = &file_path.fmt(&diff.text);
            editor(
                Path::new(path),
                Some(diff.file_line_of_first_diff(*file_i, *hunk_i) as u32),
            )
        }
        ItemData::HunkLine {
            diff,
            file_i,
            hunk_i,
            line_i,
            ..
        } => {
            let file_path = &diff.file_diffs[*file_i].header.new_file;
            let path: &str = &file_path.fmt(&diff.text);
            editor(
                Path::new(path),
                Some(diff.hunk_line_new_file_num(*file_i, *hunk_i, *line_i)),
            )
        }
        _ => None,
    }
}

fn goto_show_screen(r: String, target: Option<(String, u32)>) -> Option<Action> {
    Some(Rc::new(move |app, term| {
        let params = app.render_params(term.size().map_err(Error::Term)?);
        // A target has to be found among the rows, so they are all drawn first.
        let params = RenderParams {
            streaming: params.streaming && target.is_none(),
            ..params
        };
        let mut screen = screen::pager::show(
            Arc::clone(&app.state.config),
            Rc::clone(&app.state.repo),
            params,
            &r,
        )
        .expect("Couldn't create screen");
        if let Some((file, line_num)) = &target {
            select_line(&mut screen, file, *line_num);
        }
        app.state.screens.push(screen);
        Ok(())
    }))
}

/// Put the cursor on `line_num` of `file` as the commit left it, or on the file
/// when the commit did not touch that line.
fn select_line(screen: &mut Screen, file: &str, line_num: u32) {
    let found = screen.select_matching(|data| {
        if let ItemData::HunkLine {
            diff,
            file_i,
            hunk_i,
            line_i,
            line_range,
            ..
        } = data
        {
            if diff.file_diffs[*file_i].header.new_file.fmt(&diff.text) != file {
                return false;
            }
            let line = &diff.hunk_content(*file_i, *hunk_i)[line_range.clone()];
            !line.starts_with('-')
                && diff.hunk_line_new_file_num(*file_i, *hunk_i, *line_i) == line_num
        } else {
            false
        }
    });
    if !found {
        screen.select_matching(|data| {
            matches!(data, ItemData::Delta { diff, file_i, .. }
                if diff.file_diffs[*file_i].header.new_file.fmt(&diff.text) == file)
        });
    }
}

fn goto_show_stash_screen(stash_ref: String) -> Option<Action> {
    Some(Rc::new(move |app, term| {
        app.state.screens.push(
            screen::show_stash::create(
                Arc::clone(&app.state.config),
                Rc::clone(&app.state.repo),
                app.render_params(term.size().map_err(Error::Term)?),
                stash_ref.clone(),
            )
            .expect("Couldn't create stash screen"),
        );
        Ok(())
    }))
}

fn is_null_oid(hash: &str) -> bool {
    hash.chars().all(|c| c == '0')
}

pub(crate) const EDITOR_VARS: [&str; 4] = ["GITU_SHOW_EDITOR", "VISUAL", "EDITOR", "GIT_EDITOR"];
fn editor(file: &Path, maybe_line: Option<u32>) -> Option<Action> {
    let file = file.to_str().unwrap().to_string();

    Some(Rc::new(move |app, term| {
        let configured_editor = EDITOR_VARS
            .into_iter()
            .find_map(|var| std::env::var(var).ok());

        let Some(editor) = configured_editor else {
            return Err(Error::NoEditorSet);
        };

        let cmd = if cfg!(windows) {
            parse_editor_command_windows(&editor, &file, maybe_line)
        } else {
            parse_editor_command(&editor, &file, maybe_line)
        };

        app.run_cmd_interactive(term, cmd)?;
        app.update_screens()
    }))
}

fn parse_editor_command(editor: &str, file: &str, maybe_line: Option<u32>) -> Command {
    let args = &editor.split_whitespace().collect::<Vec<_>>();
    let mut cmd = Command::new(args[0]);

    cmd.args(&args[1..]);
    cmd.args(line_args(file, maybe_line, args[0].to_lowercase()));
    cmd
}

fn parse_editor_command_windows(editor: &str, file: &str, maybe_line: Option<u32>) -> Command {
    let args = &editor.split_whitespace().collect::<Vec<_>>();
    let mut cmd = Command::new("cmd");
    cmd.arg("/C");
    cmd.arg(args[0]);

    cmd.args(&args[1..]);
    cmd.args(line_args(file, maybe_line, args[0].to_lowercase()));
    cmd
}

fn line_args(file: &str, maybe_line: Option<u32>, lower: String) -> Vec<String> {
    if let Some(line) = maybe_line {
        if lower.ends_with("vi")
            || lower.ends_with("vim")
            || lower.ends_with("nvim")
            || lower.ends_with("nano")
            || lower.ends_with("micro")
            || lower.ends_with("nvr")
            || lower.ends_with("kak")
        {
            vec![format!("+{line}"), file.to_string()]
        } else {
            vec![format!("{file}:{line}")]
        }
    } else {
        vec![file.to_string()]
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    #[test]
    fn parse_editor_command_test() {
        let cmd = super::parse_editor_command("/bin/nAnO -f", "README.md", Some(42));
        assert_eq!(cmd.get_program(), OsStr::new("/bin/nAnO"));
        assert_eq!(
            &cmd.get_args().collect::<Vec<_>>(),
            &["-f", "+42", "README.md"]
        );
    }

    #[test]
    fn parse_editor_command_test_windows() {
        let cmd = super::parse_editor_command_windows("/bin/nAnO -f", "README.md", Some(42));
        assert_eq!(cmd.get_program(), OsStr::new("cmd"));
        assert_eq!(
            &cmd.get_args().collect::<Vec<_>>(),
            &["/C", "/bin/nAnO", "-f", "+42", "README.md"]
        );
    }
}
