use super::{Action, OpTrait};
use crate::{
    app::{App, PromptParams, State},
    error::Error,
    item_data::ItemData,
    menu::PendingMenu,
    picker::{PickerData, PickerItem, PickerState},
    screen::NavMode,
    term::Term,
};
use std::rc::Rc;

pub(crate) struct Quit;
impl OpTrait for Quit {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, term| {
            let menu = app
                .state
                .pending_menu
                .as_ref()
                .map(|pending_menu| pending_menu.menu);

            if menu == app.base_menu() {
                if app.state.screens.len() == 1 {
                    if app.state.config.general.confirm_quit.enabled {
                        app.confirm(term, "Really quit? (y or n)")?;
                    };

                    app.state.quit = true;
                } else {
                    app.state.screens.pop();
                }
            }

            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Quit/Close".into()
    }
}

pub(crate) struct OpenMenu(pub crate::menu::Menu);
impl OpTrait for OpenMenu {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        let submenu = self.0;
        Some(Rc::new(move |app, _term| {
            app.state.pending_menu = Some(PendingMenu::init(submenu));
            app.inhibit_close_menu();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Submenu".into()
    }
}

pub(crate) struct Refresh;
impl OpTrait for Refresh {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| app.update_screens()))
    }

    fn display(&self, _state: &State) -> String {
        "Refresh".into()
    }
}

pub(crate) struct ToggleArg(pub String);
impl OpTrait for ToggleArg {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        let arg_name = self.0.clone();
        Some(Rc::new(move |app, term| {
            let mut need_prompt = None;
            let mut default = None;

            let maybe_entry = if let Some(menu) = &mut app.state.pending_menu {
                Some(menu.args.entry(arg_name.clone().into()))
            } else {
                None
            };

            if let Some(entry) = maybe_entry {
                entry.and_modify(|arg| {
                    if arg.is_active() {
                        arg.unset();
                    } else if arg.expects_value() {
                        default = arg.default_as_string();
                        need_prompt = Some(arg.display);
                    } else {
                        arg.set("").expect("Should succeed");
                    }
                });
            }

            let arg_name = arg_name.clone();
            let parse_and_set_arg =
                Box::new(move |app: &mut App, _term: &mut Term, value: &str| {
                    if let Some(menu) = &mut app.state.pending_menu
                        && let Some(entry) = menu.args.get_mut(arg_name.as_str())
                    {
                        return entry.set(value);
                    }

                    Ok(())
                });

            if let Some(display) = need_prompt {
                let arg = app.prompt(
                    term,
                    &PromptParams {
                        prompt: display,
                        create_default_value: Box::new(move |_| default.clone()),
                        hide_menu: false,
                        prefill: false,
                    },
                )?;

                parse_and_set_arg(app, term, &arg)?;
            }

            app.inhibit_close_menu();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        self.0.clone()
    }
}

pub(crate) struct ToggleSection;
impl OpTrait for ToggleSection {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        if target.is_section() {
            Some(Rc::new(|app, _term| {
                app.screen_mut().toggle_section();
                Ok(())
            }))
        } else {
            None
        }
    }

    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, state: &State) -> String {
        let item = state.screens.last().unwrap().get_selected_item();
        if state.screens.last().unwrap().is_collapsed(item) {
            "Unfold".into()
        } else {
            "Fold".into()
        }
    }
}

/// Ask git for a different amount of the file around each change: a number of
/// lines, or the whole enclosing function.
pub(crate) struct DiffContext;
impl OpTrait for DiffContext {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let current = app.state.context.clone();
            let answer = app.prompt(
                term,
                &PromptParams {
                    prompt: "",
                    create_default_value: Box::new(move |_| {
                        Some(match current.as_deref() {
                            Some("-W") => "W".to_owned(),
                            Some(flag) => flag.trim_start_matches("-U").to_owned(),
                            None => DEFAULT_LINES.to_owned(),
                        })
                    }),
                    hide_menu: false,
                    prefill: false,
                },
            )?;

            let Some(flag) = context_flag(&answer) else {
                app.display_error(format!("Not a number of lines, nor W: {answer}"));
                return Ok(());
            };

            app.state.context = flag;
            app.rerender_screens()
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Diff context".into()
    }
}

/// Edit the question git is being asked. Everything a view is — the revs, what
/// is compared with what, which paths, how much context — is in that command,
/// so this is the general case of which [`DiffContext`] is one canned edit.
pub(crate) struct EditGitCommand;
impl OpTrait for EditGitCommand {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let Some(git) = app.screen().git_command.clone() else {
                app.display_error("This view is not something gitu asked git for");
                return Ok(());
            };

            let line = git.borrow().line(app.state.context.as_deref());
            let answer = app.prompt(
                term,
                &PromptParams {
                    prompt: "",
                    create_default_value: Box::new(move |_| Some(line.clone())),
                    prefill: true,
                    ..Default::default()
                },
            )?;

            let edited = git.borrow().edited(&answer);
            match edited {
                Ok(edited) => *git.borrow_mut() = edited,
                Err(err) => {
                    app.display_error(err.to_string());
                    return Ok(());
                }
            }

            // The edit said what context it wants, in the line it was given.
            app.state.context = None;
            app.rerender_screens()
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Edit git command".into()
    }
}

/// What git gives without being asked, so asking for it is asking for nothing.
const DEFAULT_LINES: &str = "3";

/// The git flag an answer asks for: `None` for the default, which is to pass
/// no flag at all. An answer that is neither a count nor the whole function
/// isn't one, since a flag that leaves no hunks leaves nothing to act on.
fn context_flag(answer: &str) -> Option<Option<Rc<str>>> {
    match answer {
        "W" | "w" => Some(Some(Rc::from("-W"))),
        DEFAULT_LINES => Some(None),
        lines if !lines.is_empty() && lines.chars().all(|c| c.is_ascii_digit()) => {
            Some(Some(Rc::from(format!("-U{lines}").as_str())))
        }
        _ => None,
    }
}

/// Fold the whole view down to its headings, or open all of it.
pub(crate) struct ToggleAllSections;
impl OpTrait for ToggleAllSections {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().toggle_all_sections();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Fold all".into()
    }
}

/// Show or hide the list of what a screen's own keymap does.
pub(crate) struct ToggleMenu;
impl OpTrait for ToggleMenu {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, _term: &mut Term| {
            let screen = app.screen_mut();
            screen.show_menu = !screen.show_menu;
            Ok(())
        }))
    }

    fn display(&self, state: &State) -> String {
        if state.screens.last().unwrap().show_menu {
            "Hide keys".into()
        } else {
            "Show keys".into()
        }
    }
}

/// Turn one of the configured renderer features on or off, re-rendering with
/// it. The diff is what changes, not the position in it: a row's identity is
/// its place in the patch, which no amount of re-rendering moves.
pub(crate) struct RendererFeatures;
impl OpTrait for RendererFeatures {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let offered = crate::diff_colorizer::offered_features(
                &app.state.config.general.diff_colorizer.features,
                &app.state.repo.config().map_err(Error::ReadGitConfig)?,
            )?;
            if offered.is_empty() {
                app.display_error("No renderer features are configured");
                return Ok(());
            }

            let items = offered
                .iter()
                .map(|feature| {
                    let mark = if app.state.features.contains(feature) {
                        "● "
                    } else {
                        "  "
                    };
                    PickerItem::new(
                        format!("{mark}{feature}"),
                        PickerData::Item(feature.clone()),
                    )
                })
                .collect();

            let picked = app.pick(term, PickerState::new("Renderer feature", items, false))?;
            let Some(picked) = picked else {
                return Ok(());
            };

            app.state.features = toggled(&app.state.features, picked.display());
            app.rerender_screens()
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Renderer features".into()
    }
}

/// `features` with `feature` removed if present and appended if not. Order is
/// otherwise kept, since delta resolves a conflict between two features in
/// favour of the later one.
fn toggled(features: &[String], feature: &str) -> Rc<[String]> {
    if features.iter().any(|held| held == feature) {
        return features
            .iter()
            .filter(|held| *held != feature)
            .cloned()
            .collect();
    }

    features
        .iter()
        .cloned()
        .chain([feature.to_owned()])
        .collect()
}

/// Name the commit under the cursor as the one [`App::pick_commit`] was after.
pub(crate) struct SelectCommit;
impl OpTrait for SelectCommit {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        let ItemData::Commit { oid, .. } = target else {
            return None;
        };
        let oid = oid.clone();

        Some(Rc::new(move |app: &mut App, _term: &mut Term| {
            app.state.picked_commit = Some(oid.clone());
            Ok(())
        }))
    }

    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, _state: &State) -> String {
        "Select".into()
    }
}

pub(crate) struct MoveUp;
impl OpTrait for MoveUp {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().select_previous(NavMode::Normal);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Up".into()
    }
}

pub(crate) struct MoveDown;
impl OpTrait for MoveDown {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().select_next(NavMode::Normal);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Down".into()
    }
}

pub(crate) struct MoveDownLine;
impl OpTrait for MoveDownLine {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().select_next(NavMode::IncludeSubLines);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Down line".into()
    }
}

pub(crate) struct MoveUpLine;
impl OpTrait for MoveUpLine {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().select_previous(NavMode::IncludeSubLines);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Up line".into()
    }
}

/// Reach the selection out over another line, towards the end of the hunk or
/// back towards its start.
pub(crate) struct ExtendSelection(pub bool);
impl OpTrait for ExtendSelection {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        let forwards = self.0;
        Some(Rc::new(move |app: &mut App, _term: &mut Term| {
            app.screen_mut().extend_selection(forwards);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        if self.0 {
            "Select down".into()
        } else {
            "Select up".into()
        }
    }
}

pub(crate) struct MoveToScreenLine(pub usize);
impl OpTrait for MoveToScreenLine {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        let screen_line = self.0;
        Some(Rc::new(move |app, _term| {
            app.screen_mut().move_cursor_to_screen_line(screen_line);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Move to line".into()
    }
}

pub(crate) struct MoveNextSection;
impl OpTrait for MoveNextSection {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            let depth = app.screen().get_selected_item().depth;
            app.screen_mut().select_next(NavMode::Siblings { depth });
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Next section".into()
    }
}

pub(crate) struct MovePrevSection;
impl OpTrait for MovePrevSection {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            let depth = app.screen().get_selected_item().depth;
            app.screen_mut()
                .select_previous(NavMode::Siblings { depth });
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Prev section".into()
    }
}

pub(crate) struct MoveParentSection;
impl OpTrait for MoveParentSection {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            let depth = app.screen().get_selected_item().depth.saturating_sub(1);
            app.screen_mut()
                .select_previous(NavMode::Siblings { depth });
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Parent section".into()
    }
}

pub(crate) struct HalfPageUp;
impl OpTrait for HalfPageUp {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().half_page_up();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Scroll half page up".into()
    }
}

pub(crate) struct HalfPageDown;
impl OpTrait for HalfPageDown {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().half_page_down();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Scroll half page down".into()
    }
}

pub(crate) struct FullPageUp;
impl OpTrait for FullPageUp {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().full_page_up();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Scroll page up".into()
    }
}

pub(crate) struct FullPageDown;
impl OpTrait for FullPageDown {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().full_page_down();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Scroll page down".into()
    }
}

pub(crate) struct ScrollViewUp;
impl OpTrait for ScrollViewUp {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.close_menu();
            app.screen_mut().scroll_view_up(1);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Scroll view up".into()
    }
}

pub(crate) struct ScrollViewDown;
impl OpTrait for ScrollViewDown {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.close_menu();
            app.screen_mut().scroll_view_down(1);
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Scroll view down".into()
    }
}

pub(crate) struct MoveTop;
impl OpTrait for MoveTop {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().move_cursor_to_top();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Top".into()
    }
}

pub(crate) struct MoveBottom;
impl OpTrait for MoveBottom {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            app.screen_mut().move_cursor_to_bottom();
            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Bottom".into()
    }
}
