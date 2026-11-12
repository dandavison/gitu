use super::{Action, OpTrait};
use crate::{
    Res,
    app::{App, PromptParams, State},
    config::ColorMode,
    diff_renderer::Theme,
    error::Error,
    item_data::ItemData,
    menu::PendingMenu,
    picker::{Picked, PickerData, PickerItem, PickerState},
    screen::NavMode,
    term::Term,
};
use std::cell::RefCell;
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
                        app.confirm(term, "Really quit? [y/n]")?;
                    };

                    app.state.quit = true;
                } else {
                    app.state.screens.pop();
                    app.screen_mut().come_back()?;
                }
            }

            Ok(())
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Quit/Close".into()
    }
}

/// Folds the section the user unfolded on this screen, if that is what is
/// selected. Otherwise closes the screen on top, as quit does, but on the first
/// screen does nothing rather than leave gitu.
pub(crate) struct FoldOrBack;
impl OpTrait for FoldOrBack {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| {
            let menu = app
                .state
                .pending_menu
                .as_ref()
                .map(|pending_menu| pending_menu.menu);

            if menu != app.base_menu() {
                return Ok(());
            }
            if app.screen().selected_unfolded_here() {
                app.screen_mut().toggle_section()?;
            } else if app.state.screens.len() > 1 {
                app.state.screens.pop();
                app.screen_mut().come_back()?;
            }

            Ok(())
        }))
    }

    fn display(&self, state: &State) -> String {
        if state.screens.last().unwrap().selected_unfolded_here() {
            "Fold".into()
        } else {
            "Back".into()
        }
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

pub(crate) struct Suspend;
impl OpTrait for Suspend {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, term| {
            term.suspend(&app.state.config).map_err(Error::Term)
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Suspend".into()
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
                        history: None,
                        bare: false,
                        on_edit: None,
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
                app.screen_mut().toggle_section()?;
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
                        current.as_deref().map(|flag| match flag {
                            "-W" => "W".to_owned(),
                            flag => flag.trim_start_matches("-U").to_owned(),
                        })
                    }),
                    hide_menu: false,
                    prefill: true,
                    history: None,
                    bare: false,
                    on_edit: None,
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

/// Widen (`true`) or narrow the context git is asked for, by a power of two: to the next power of
/// two above the context, or below it, down to none. From the whole function, there is nothing
/// to step from.
pub(crate) struct StepDiffContext(pub(crate) bool);
impl OpTrait for StepDiffContext {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        let wider = self.0;
        Some(Rc::new(move |app: &mut App, _term: &mut Term| {
            let lines = match app.state.context.as_deref() {
                Some("-W") => {
                    app.display_error("The context is the whole function; U sets a number");
                    return Ok(());
                }
                Some(flag) => flag.trim_start_matches("-U").parse().unwrap_or(3),
                // git's own context, as configured or by default.
                None => app
                    .state
                    .repo
                    .config()
                    .and_then(|config| config.get_i32("diff.context"))
                    .map_or(3, |lines| lines.max(0) as u32),
            };
            let lines = if wider {
                (lines + 1).next_power_of_two()
            } else if lines <= 1 {
                0
            } else {
                // The largest power of two below `lines`.
                1 << (u32::BITS - 1 - (lines - 1).leading_zeros())
            };
            app.state.context = Some(Rc::from(format!("-U{lines}").as_str()));
            app.rerender_screens()
        }))
    }

    fn display(&self, _state: &State) -> String {
        if self.0 {
            "Wider context".into()
        } else {
            "Narrower context".into()
        }
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
                app.display_error(NOT_ASKED_FOR);
                return Ok(());
            };

            let line = git.borrow().line(app.state.context.as_deref());
            let answer = app.prompt(
                term,
                &PromptParams {
                    prompt: "",
                    create_default_value: Box::new(move |_| Some(line.clone())),
                    prefill: true,
                    history: Some(crate::prompt::HistoryKind::GitCommand),
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

/// Limit the view to some of the files it covers, or drop some of them. Both
/// are edits of the same command, and so of the same kind as [`EditGitCommand`]
/// — this only saves writing the pathspec magic out.
pub(crate) struct FilePatterns;
impl OpTrait for FilePatterns {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            let Some(git) = app.screen().git_command.clone() else {
                app.display_error(NOT_ASKED_FOR);
                return Ok(());
            };

            let patterns = git.borrow().file_patterns().join(" ");
            let answer = app.prompt(
                term,
                &PromptParams {
                    prompt: "File filters",
                    create_default_value: Box::new(move |_| Some(patterns.clone())),
                    prefill: true,
                    history: Some(crate::prompt::HistoryKind::FilePatterns),
                    ..Default::default()
                },
            )?;

            let patterns = match shell_words::split(&answer) {
                Ok(patterns) => patterns,
                Err(_) => {
                    app.display_error(Error::EditedCommandQuotes.to_string());
                    return Ok(());
                }
            };

            ask_for(app, &git, &patterns)
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Files".into()
    }
}

/// Drop the file whose header is under the cursor from the view, as if it were
/// not in the patch. The frequent case, and the one worth a keystroke: the noise is in
/// front of you and it goes away.
pub(crate) struct HideFile;
impl OpTrait for HideFile {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        let path = file_of(target)?;

        Some(Rc::new(move |app: &mut App, _term: &mut Term| {
            let Some(git) = app.screen().git_command.clone() else {
                app.display_error(NOT_ASKED_FOR);
                return Ok(());
            };

            let mut patterns = git.borrow().file_patterns();
            patterns.push(crate::calling_process::excluding(&path));

            ask_for(app, &git, &patterns)
        }))
    }

    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, _state: &State) -> String {
        "Hide file".into()
    }
}

const NOT_ASKED_FOR: &str = "This view is not something gitu asked git for";

/// Put the held command again, for these paths, and remember it as if typed
/// at `:`.
fn ask_for(
    app: &mut App,
    git: &Rc<RefCell<crate::calling_process::GitCommand>>,
    patterns: &[String],
) -> Res<()> {
    let asking_for = git.borrow().asking_for(patterns);
    app.state.prompt.remember(
        crate::prompt::HistoryKind::GitCommand,
        &asking_for.line(app.state.context.as_deref()),
    )?;
    *git.borrow_mut() = asking_for;
    app.rerender_screens()
}

/// The file a row is the header of.
fn file_of(target: &ItemData) -> Option<String> {
    let ItemData::Delta { diff, file_i, .. } = target else {
        return None;
    };

    let header = &diff.file_diffs[*file_i].header;
    let path = match header.status {
        crate::gitu_diff::Status::Deleted => &header.old_file,
        _ => &header.new_file,
    };

    Some(path.fmt(&diff.text).into_owned())
}

/// The git flag an answer asks for: `None` for no answer, which is to pass no
/// flag at all and leave the context to the command and git's config. An
/// answer that is neither a count nor the whole function isn't one, since a
/// flag that leaves no hunks leaves nothing to act on.
fn context_flag(answer: &str) -> Option<Option<Rc<str>>> {
    match answer {
        "" => Some(None),
        "W" | "w" => Some(Some(Rc::from("-W"))),
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
            let offered = crate::diff_renderer::offered_features(
                &app.state.config.general.diff_renderer.features,
                &app.state.repo.config().map_err(Error::ReadGitConfig)?,
            )?;
            let general = &app.state.config.general;
            let commands: Vec<String> = general.commands.keys().cloned().collect();
            let views: Vec<String> = general.views.keys().cloned().collect();
            if offered.is_empty() && commands.is_empty() && views.is_empty() {
                app.display_error("No renderer features are configured");
                return Ok(());
            }
            // A column each: a feature's key toggles it, a command's runs it,
            // and a view's opens it.
            let names: Vec<&String> = offered.iter().chain(&commands).chain(&views).collect();
            let columns: Vec<usize> = [(0, offered.len()), (1, commands.len()), (2, views.len())]
                .into_iter()
                .flat_map(|(column, len)| std::iter::repeat_n(column, len))
                .collect();
            let is_command = |name: &str| commands.iter().any(|command| command == name);
            let is_view = |name: &str| views.iter().any(|view| view == name);

            let mut cursor = 0;
            loop {
                let items = feature_items(app, &offered)
                    .into_iter()
                    .chain(named_items(app, &commands))
                    .chain(named_items(app, &views))
                    .collect();
                let picker = PickerState::new("Features", items, false)
                    .assigning_keys()
                    .in_columns(vec!["Features", "Commands", "Views"], columns.clone())
                    .with_cursor(cursor);
                match app.pick_or_set_key(term, picker)? {
                    None => return Ok(()),
                    Some(Picked::Item(picked)) if is_command(picked.display()) => {
                        return crate::ops::command::run(app, term, picked.display());
                    }
                    Some(Picked::Item(picked)) if is_view(picked.display()) => {
                        return crate::ops::command::open_view(app, term, picked.display());
                    }
                    Some(Picked::Item(picked)) => return toggle_feature(app, picked.display()),
                    Some(Picked::KeySet(key, picked)) => {
                        let name = picked.display();
                        cursor = names.iter().position(|held| *held == name).unwrap_or(0);
                        // Space enables or disables a feature and keeps the list open; any
                        // other key becomes the feature's, command's or view's key.
                        if key != ' ' {
                            set_feature_key(app, name, key);
                        } else if !is_command(name) && !is_view(name) {
                            toggle_feature(app, name)?;
                        }
                    }
                }
            }
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Hotkeys".into()
    }
}

/// Move through the themes defined in git config, rendering the screen in view
/// with each, shown in one row so that the screen stays in view. Enter keeps
/// the theme and saves it; cancelling restores the one there was.
pub(crate) struct RendererThemes;
impl OpTrait for RendererThemes {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, term: &mut Term| {
            // The git config the renderer reads, since a theme it cannot find would change
            // nothing.
            let dir = app.state.repo.workdir().unwrap_or(app.state.repo.path());
            let git_config = match app.state.config.general.diff_renderer.git_config.open(dir) {
                Some(Some(git_config)) => git_config,
                Some(None) => git2::Config::new().map_err(Error::ReadGitConfig)?,
                None => app.state.repo.config().map_err(Error::ReadGitConfig)?,
            };
            let themes = crate::diff_renderer::themes(
                &git_config,
                &app.state.config.general.diff_renderer.builtin_themes,
            )?;
            if themes.is_empty() {
                app.display_error("No themes are defined in git config");
                return Ok(());
            }

            // Those for the mode delta renders in first, the others saying which
            // mode they are for, in the list only. A theme that does not say is
            // for either.
            let mode = app.state.config.general.diff_renderer.color_mode;
            let other =
                |theme: &Theme| mode.is_some() && theme.mode.is_some_and(|m| Some(m) != mode);
            let (others, matching): (Vec<Theme>, Vec<Theme>) =
                themes.into_iter().partition(|theme| other(theme));
            let themes: Vec<Theme> = matching.into_iter().chain(others).collect();

            let entered = app.state.theme.clone();
            let cursor = entered
                .as_ref()
                .and_then(|theme| themes.iter().position(|held| &held.name == theme))
                .map_or(0, |i| i + 1);
            // The first item is no theme: the user's own configuration as it is.
            let items = std::iter::once(PickerItem::new(NO_THEME, PickerData::Item(String::new())))
                .chain(themes.iter().map(|theme| {
                    let display = match theme.mode {
                        Some(ColorMode::Dark) if other(theme) => format!("{} (dark)", theme.name),
                        Some(ColorMode::Light) if other(theme) => format!("{} (light)", theme.name),
                        _ => theme.name.clone(),
                    };
                    let item = PickerItem::new(display, PickerData::Item(theme.name.clone()));
                    if other(theme) {
                        item.only_in_list()
                    } else {
                        item
                    }
                }))
                .collect();
            let picker = PickerState::new("Theme", items, false)
                .one_row()
                .with_cursor(cursor);

            let picked = app.pick_previewing(term, picker, &mut |app, data| {
                app.state.theme = theme_of(data);
                app.rerender_screen()
            })?;

            match picked {
                Some(data) => {
                    app.state.theme = theme_of(&data);
                    let dir = app.state.repo.workdir().unwrap_or(app.state.repo.path());
                    if !app.state.read_only
                        && let Err(e) =
                            crate::config_edit::set_theme(dir, app.state.theme.as_deref())
                    {
                        app.display_error(e.to_string());
                    }
                }
                None => app.state.theme = entered,
            }
            app.rerender_screens()
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Themes".into()
    }
}

const NO_THEME: &str = "(no theme)";

fn theme_of(data: &PickerData) -> Option<String> {
    Some(data.display().to_owned()).filter(|theme| !theme.is_empty())
}

/// Each feature, marked if it is enabled, beside its key if it has one.
fn feature_items(app: &App, offered: &[String]) -> Vec<PickerItem> {
    let enabled = enabled_by_config(app);
    offered
        .iter()
        .map(|feature| {
            let mark = if is_enabled(app, feature, &enabled) {
                '●'
            } else {
                ' '
            };
            let key = app.state.feature_keys.get(feature).copied().unwrap_or(' ');
            PickerItem::new(
                format!("{mark} {key} {feature}"),
                PickerData::Item(feature.clone()),
            )
        })
        .collect()
}

/// Each of `names`, a command or view, beside its key if it has one.
fn named_items(app: &App, names: &[String]) -> Vec<PickerItem> {
    names
        .iter()
        .map(|name| {
            let key = app.state.feature_keys.get(name).copied().unwrap_or(' ');
            PickerItem::new(format!("{key} {name}"), PickerData::Item(name.clone()))
        })
        .collect()
}

pub(crate) fn toggle_feature(app: &mut App, feature: &str) -> Res<()> {
    let enabled = enabled_by_config(app);
    app.state.features = toggled(
        &app.state.features,
        feature,
        is_enabled_by_config(feature, &enabled),
    );
    app.rerender_screens()
}

/// Whether `feature` is enabled: enabled in-session, or enabled by the user's own
/// configuration (`enabled`) and not disabled in-session. Whether it is then in
/// effect is delta's to decide, from the rest of the user's configuration, such
/// as a minimum width.
fn is_enabled(app: &App, feature: &str, enabled: &[String]) -> bool {
    let held = |entry: &str| app.state.features.iter().any(|held| held == entry);
    held(feature) || (is_enabled_by_config(feature, enabled) && !held(&format!("-{feature}")))
}

/// Whether `feature` is among `enabled`, which are by delta's name.
fn is_enabled_by_config(feature: &str, enabled: &[String]) -> bool {
    let deltas_name = crate::diff_renderer::deltas_name(feature);
    enabled.contains(&deltas_name)
}

/// The features the user's own configuration enables now, by delta's name.
fn enabled_by_config(app: &App) -> Vec<String> {
    let dir = app.state.repo.workdir().unwrap_or(app.state.repo.path());
    app.state
        .config
        .general
        .diff_renderer
        .enabled_features
        .get(dir)
}

/// Make `key` toggle `feature`, or run the command of that name, at the top
/// level, in this session and, by saving it to the config file, the next.
/// `feature`'s own key unsets it, and a key bound already is refused.
fn set_feature_key(app: &mut App, feature: &str, key: char) {
    let key = if app.state.feature_keys.get(feature) == Some(&key) {
        None
    } else if app.state.config.bindings.binds_at_top_level(key) {
        app.display_error(format!("{key} is bound already"));
        return;
    } else {
        Some(key)
    };

    app.state
        .feature_keys
        .retain(|name, held| name != feature && Some(*held) != key);
    if let Some(key) = key {
        app.state.feature_keys.insert(feature.to_owned(), key);
    }

    if app.state.read_only {
        return;
    }
    let dir = app.state.repo.workdir().unwrap_or(app.state.repo.path());
    if let Err(e) = crate::config_edit::set_feature_keys(dir, &app.state.feature_keys) {
        app.display_error(e.to_string());
    }
}

/// `features` with `feature` toggled. Its entry enables it, or, if the user's own
/// configuration enables it (`enabled_by_config`), disables it as `-feature`; the
/// entry is removed if present, returning to the user's configuration, and
/// appended if not. Order is otherwise kept, since delta resolves a conflict
/// between two features in favour of the later one.
fn toggled(features: &[String], feature: &str, enabled_by_config: bool) -> Rc<[String]> {
    let entry = if enabled_by_config {
        format!("-{feature}")
    } else {
        feature.to_owned()
    };
    if features.contains(&entry) {
        return features
            .iter()
            .filter(|held| **held != entry)
            .cloned()
            .collect();
    }

    features.iter().cloned().chain([entry]).collect()
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
