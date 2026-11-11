use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Read;
use std::io::Write;
use std::ops::DerefMut;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use git2::Repository;

use crate::cli;
use crate::cmd_log::CmdLog;
use crate::cmd_log::CmdLogEntry;
use crate::config::Config;
use crate::error::Error;
use crate::file_watcher::FileWatcher;
use crate::item_data::Rev;
use crate::items::RenderParams;
use crate::menu::Menu;
use crate::menu::PendingMenu;
use crate::ops::Op;
use crate::picker::PickerData;
use crate::picker::PickerState;
use crate::picker::{Picked, PickerStatus};
use crate::prompt;
use crate::rebase_todo::RebaseTodo;
use crate::screen;
use crate::screen::Screen;
use crate::screen::pager::Paged;
use crate::term::Term;
use crate::text_input;
use crate::ui;

use super::Res;

pub(crate) struct State {
    pub repo: Rc<Repository>,
    pub config: Arc<Config>,
    pub pending_keys: Vec<(KeyModifiers, KeyCode)>,
    pub quit: bool,
    /// What gitu exits with. Non-zero only to tell git, which is waiting on
    /// gitu as its sequence editor, to call the rebase off: it is until the
    /// list is accepted.
    pub exit_code: i32,
    pub screens: Vec<Screen>,
    pub pending_menu: Option<PendingMenu>,
    pending_cmd: Option<(Child, Arc<RwLock<CmdLogEntry>>)>,
    enable_async_cmds: bool,
    pub debug: bool,
    pub current_cmd_log: CmdLog,
    pub prompt: prompt::Prompt,
    pub picker: Option<PickerState>,
    /// The commit `select_commit` named, read by [`App::pick_commit`].
    pub picked_commit: Option<String>,
    /// Renderer features chosen in-session, overlaying the user's own
    /// configuration. Never written to disk: quitting returns them to it.
    pub features: Rc<[String]>,
    /// The theme overlaid after `features`, as configured and as chosen since.
    pub theme: Option<String>,
    /// How much of a file to ask git for around each change (`-U8`, `-W`),
    /// chosen in-session. `None` is git's own default.
    pub context: Option<Rc<str>>,
    /// The keys that toggle renderer features, as configured and as set since.
    pub feature_keys: BTreeMap<String, char>,
    pub clipboard: Option<Clipboard>,
    /// Whether ops that write are hidden and refused, and nothing chosen
    /// in-session is saved ([`crate::config::ReadOnly`]).
    pub read_only: bool,
    needs_redraw: bool,
    /// When to render every screen again at the terminal's new width: once no resize has come
    /// for [`RESIZE_RENDER_DELAY`], so that dragging a window's edge runs the renderer once.
    rerender_after_resize: Option<Instant>,
    file_watcher: Option<FileWatcher>,
    inhibit_close_menu: bool,
}

pub struct App {
    pub(crate) state: State,
}

impl App {
    pub fn create(
        repo: Rc<Repository>,
        size: (u16, u16),
        args: &cli::Args,
        config: Arc<Config>,
        enable_async_cmds: bool,
        piped: Option<Paged>,
    ) -> Res<Self> {
        let params = RenderParams {
            size,
            features: overlay(&[], config.general.diff_renderer.theme.as_deref()),
            context: None,
            streaming: enable_async_cmds,
        };
        let screens = match args.command {
            // `--pager`: git's output is already in hand, with the command it ran.
            _ if let Some(paged) = piped => {
                vec![screen::pager::create(
                    Arc::clone(&config),
                    Rc::clone(&repo),
                    params,
                    paged,
                )?]
            }
            Some(cli::Commands::Show { ref reference }) => {
                vec![screen::pager::show(
                    Arc::clone(&config),
                    Rc::clone(&repo),
                    params.clone(),
                    reference,
                )?]
            }
            Some(cli::Commands::Blame { ref rev, ref file }) => {
                vec![screen::blame::create(
                    Arc::clone(&config),
                    Rc::clone(&repo),
                    params.clone(),
                    file.clone(),
                    rev.clone(),
                    None,
                )?]
            }
            Some(cli::Commands::Rebase { ref upstream }) => {
                let todo = RebaseTodo::capture(&repo, OsStr::new(upstream), &[])?;
                vec![rebase_todo_screen(&config, &repo, params.clone(), todo)?]
            }
            Some(cli::Commands::SequenceEditor { ref file }) => {
                let todo = RebaseTodo::read(&repo, file)?;
                vec![rebase_todo_screen(&config, &repo, params.clone(), todo)?]
            }
            None => vec![screen::status::create(
                Arc::clone(&config),
                Rc::clone(&repo),
                params,
            )?],
        };

        let clipboard = Clipboard::new()
            .inspect_err(|e| log::warn!("Couldn't initialize clipboard: {e}"))
            .ok();
        let read_only = config.general.read_only.is_on(&repo);
        let prompt = prompt::Prompt::with_history(repo.path().join("gitu"), read_only);

        let mut app = Self {
            state: State {
                repo,
                feature_keys: config.general.diff_renderer.feature_keys.clone(),
                theme: config.general.diff_renderer.theme.clone(),
                config,
                pending_keys: vec![],
                enable_async_cmds,
                debug: args.debug,
                quit: false,
                exit_code: match args.command {
                    Some(cli::Commands::SequenceEditor { .. }) => 1,
                    _ => 0,
                },
                screens,
                pending_cmd: None,
                pending_menu: None,
                current_cmd_log: CmdLog::new(),
                prompt,
                picker: None,
                picked_commit: None,
                features: Rc::from([]),
                context: None,
                clipboard,
                read_only,
                file_watcher: None,
                needs_redraw: true,
                rerender_after_resize: None,
                inhibit_close_menu: false,
            },
        };

        // The menu a screen imposes is in force from the first key on, not from
        // the first key that closes a menu.
        app.close_menu();
        app.state.file_watcher = app.init_file_watcher()?;
        Ok(app)
    }

    fn init_file_watcher(&mut self) -> Res<Option<FileWatcher>> {
        if !self.state.config.general.refresh_on_file_change.enabled {
            return Ok(None);
        }

        let hiding_untracked_files = !self
            .state
            .repo
            .config()
            .map_err(Error::ReadGitConfig)?
            .get_bool("status.showUntrackedFiles")
            .unwrap_or(true);

        if hiding_untracked_files {
            self.display_info("File watcher disabled (status.showUntrackedFiles is off)");

            return Ok(None);
        }

        Ok(
            FileWatcher::new(self.state.repo.workdir().expect("Bare repos unhandled"))
                .inspect_err(|err| {
                    self.display_error(err.to_string());
                    self.display_info("File watcher disabled");
                })
                .ok(),
        )
    }

    pub fn run(&mut self, term: &mut Term, max_tick_delay: Duration) -> Res<()> {
        while !self.state.quit {
            let event = if term.poll_event(max_tick_delay)? {
                Some(term.read_event()?)
            } else if term.hung_up() {
                return Ok(());
            } else {
                None
            };

            self.update(term, event)?;
        }

        Ok(())
    }

    pub fn update(&mut self, term: &mut Term, event: Option<Event>) -> Res<()> {
        if let Some(e) = event {
            self.handle_event(term, e)?;
        }

        if let Some(at) = self.state.rerender_after_resize
            && Instant::now() >= at
            && !term.has_pending_event()?
        {
            self.state.rerender_after_resize = None;
            self.rerender_screens()?;
        }

        if let Some(file_watcher) = &mut self.state.file_watcher
            && file_watcher.pending_updates()
        {
            self.update_screens()?;
        }

        let finished = self.finish_screens();
        self.handle_result(finished)?;

        let handle_pending_cmd_result = self.handle_pending_cmd();
        self.handle_result(handle_pending_cmd_result)?;

        if self.state.needs_redraw {
            self.redraw_now(term)?;
        }

        Ok(())
    }

    /// Give each screen whose rows were still being drawn all of them, once
    /// they are.
    fn finish_screens(&mut self) -> Res<()> {
        let mut finished = false;
        for screen in &mut self.state.screens {
            finished |= screen.finish()?;
        }
        if finished {
            self.stage_redraw();
        }
        Ok(())
    }

    pub fn update_screens(&mut self) -> Res<()> {
        self.rebuild_screens(Screen::refresh)
    }

    /// Rebuild every screen, leaving each cursor where it was in the diff rather
    /// than on the line number it happened to occupy. For a re-render of the
    /// same content (a renderer feature turned on or off), the rows move but the
    /// place in the patch does not.
    pub fn rerender_screens(&mut self) -> Res<()> {
        self.rebuild_screens(Screen::refresh_keeping_position)
    }

    /// [`App::rerender_screens`] for the screen in view only, as a preview: the
    /// screens under it keep what they were rendered with.
    pub(crate) fn rerender_screen(&mut self) -> Res<()> {
        let features = self.overlay();
        let screen = self.state.screens.last_mut().unwrap();
        screen.features = features;
        screen.refresh_keeping_position()?;
        self.stage_redraw();
        Ok(())
    }

    /// The features to render with: those chosen in-session, then the theme.
    fn overlay(&self) -> Rc<[String]> {
        overlay(&self.state.features, self.state.theme.as_deref())
    }

    fn rebuild_screens(&mut self, rebuild: impl Fn(&mut Screen) -> Res<()>) -> Res<()> {
        let features = self.overlay();
        for screen in &mut self.state.screens {
            screen.features = Rc::clone(&features);
            screen.context = self.state.context.clone();
            rebuild(screen)?;
        }

        self.stage_redraw();
        Ok(())
    }

    pub fn handle_event(&mut self, term: &mut Term, event: Event) -> Res<()> {
        match event {
            Event::Resize(w, h) => {
                // The renderer lays rows out at the width, so a new width renders them again; until
                // then the rows it drew are fitted to the new size.
                let width_changed = self.state.screens.iter().any(|screen| screen.size.0 != w);
                for screen in self.state.screens.iter_mut() {
                    screen.resize(w, h)?;
                }
                if width_changed {
                    let delay = if self.state.enable_async_cmds {
                        RESIZE_RENDER_DELAY
                    } else {
                        Duration::ZERO
                    };
                    self.state.rerender_after_resize = Some(Instant::now() + delay);
                }

                self.stage_redraw();
                Ok(())
            }
            Event::Mouse(ref mouse) => {
                if self.state.config.general.mouse_support
                    && self.handle_mouse_input(term, mouse)?
                {
                    self.stage_redraw();
                }
                Ok(())
            }
            Event::Key(mut key) => {
                if self.state.pending_cmd.is_none() {
                    self.state.current_cmd_log.clear();
                }

                if self.state.pending_cmd.is_none() {
                    self.state.current_cmd_log.clear();
                }

                // The key received changes as shift is pressed: '/' becomes '?'
                // on a US keyboard, and tab becomes backtab. The modifier is
                // redundant there, and on the other keys it is the only sign
                // shift was held.
                if matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
                    key.modifiers = key.modifiers.difference(KeyModifiers::SHIFT);
                }

                if self.state.picker.is_some() {
                    self.handle_picker_input(key)?;
                } else if self.state.prompt.state.focused {
                    self.state.prompt.handle_key(key);
                } else {
                    self.handle_key_input(term, key)?;
                }

                self.stage_redraw();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn redraw_now(&mut self, term: &mut Term) -> Res<()> {
        if self.state.screens.last_mut().is_some() {
            ui::ui(term, &mut self.state)?;
            self.state.needs_redraw = false;
        };

        Ok(())
    }

    /// Prints the screen, and says so, if it fits on the terminal with a row
    /// to spare for the shell's prompt: less's `--quit-if-one-screen`. Waits
    /// for the rest of a screen still being drawn while what there is fits.
    pub fn print_if_one_screen(&mut self, term: &mut Term) -> Res<bool> {
        let screen = self.state.screens.last_mut().unwrap();
        let (_, rows) = term.size().map_err(Error::Term)?;
        let rows = rows.saturating_sub(1).into();
        loop {
            if !screen.fits_in(rows) {
                return Ok(false);
            }
            if !screen.is_drawing() {
                break;
            }
            if !screen.finish()? {
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        screen::print_screen(term, screen)?;
        Ok(true)
    }

    pub fn stage_redraw(&mut self) {
        self.state.needs_redraw = true;
    }

    fn handle_key_input(&mut self, term: &mut Term, key: event::KeyEvent) -> Res<()> {
        if key.kind != event::KeyEventKind::Press {
            return Ok(());
        }

        let menu = match &self.state.pending_menu {
            None => self.base_menu().unwrap_or(Menu::Root),
            Some(menu) if menu.menu == Menu::Help => Menu::Root,
            Some(menu) => menu.menu,
        };

        self.state.pending_keys.push((key.modifiers, key.code));

        let matching_bindings = self
            .state
            .config
            .bindings
            .match_bindings(&menu, &self.state.pending_keys)
            .collect::<Vec<_>>();

        match matching_bindings[..] {
            [binding] => {
                if binding.keys == self.state.pending_keys {
                    let op = binding.op.clone();
                    self.state.pending_keys.clear();
                    self.handle_op(op, term)?;
                }
            }
            [] => {
                let feature = match self.state.pending_keys[..] {
                    [(KeyModifiers::NONE, KeyCode::Char(key))] if menu == Menu::Root => {
                        self.feature_with_key(key)
                    }
                    _ => None,
                };
                self.state.pending_keys.clear();
                if let Some(feature) = feature {
                    crate::ops::editor::toggle_feature(self, &feature)?;
                }
            }
            [_, ..] => (),
        }

        Ok(())
    }

    fn handle_mouse_input(&mut self, term: &mut Term, mouse: &MouseEvent) -> Res<bool> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let click_y = mouse.row as usize;
                if self.screen().is_valid_screen_line(click_y) {
                    let old_selected_item_id = self.screen().get_selected_item().id;
                    self.handle_op(Op::MoveToScreenLine(click_y), term)?;
                    let new_selected_item = self.screen().get_selected_item();

                    if old_selected_item_id == new_selected_item.id {
                        // If the item clicked was already the current item, then try to
                        // toggle it if it's a section or show it.
                        if new_selected_item.data.is_section() {
                            self.handle_op(Op::ToggleSection, term)?;
                        } else {
                            self.handle_op(Op::Show, term)?;
                        }
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                let click_y = mouse.row as usize;
                if self.screen().is_valid_screen_line(click_y) {
                    self.handle_op(Op::MoveToScreenLine(click_y), term)?;
                    self.handle_op(Op::Show, term)?;
                }
            }
            MouseEventKind::ScrollUp => {
                let scroll_lines = self.state.config.general.mouse_scroll_lines;
                if scroll_lines > 0 {
                    self.screen_mut().scroll_view_up(scroll_lines);
                }
            }
            MouseEventKind::ScrollDown => {
                let scroll_lines = self.state.config.general.mouse_scroll_lines;
                if scroll_lines > 0 {
                    self.screen_mut().scroll_view_down(scroll_lines);
                }
            }
            _ => return Ok(false),
        }

        Ok(true)
    }

    pub(crate) fn handle_op(&mut self, op: Op, term: &mut Term) -> Res<()> {
        if self.state.read_only && op.writes() {
            return Ok(());
        }
        let item_data = self.screen().selected_target();
        let implementation = op.clone().implementation();

        if let Some(mut action) = implementation.get_action(&item_data) {
            let result = Rc::get_mut(&mut action).unwrap()(self, term);
            self.handle_result(result)?;
            if !self.state.inhibit_close_menu {
                self.close_menu();
            }
            self.state.inhibit_close_menu = false;
        }

        Ok(())
    }

    fn handle_result<T>(&mut self, result: Res<T>) -> Res<()> {
        match result {
            Ok(_) => Ok(()),
            Err(Error::NoMoreEvents) => Err(Error::NoMoreEvents),
            Err(Error::PromptAborted) => Ok(()),
            Err(error) => {
                self.state
                    .current_cmd_log
                    .push(CmdLogEntry::Error(error.to_string()));

                Ok(())
            }
        }
    }

    /// What a screen opened now should render with: the viewport it will get,
    /// and the features currently selected.
    pub(crate) fn render_params(&self, size: (u16, u16)) -> RenderParams {
        RenderParams {
            size,
            features: self.overlay(),
            context: self.state.context.clone(),
            streaming: self.state.enable_async_cmds,
        }
    }

    pub fn close_menu(&mut self) {
        let hide = self.screen().menu.is_some() && !self.screen().show_menu;
        self.state.pending_menu = self.base_menu().map(|menu| PendingMenu {
            is_hidden: hide,
            ..PendingMenu::init(menu)
        });
    }

    /// The menu to fall back to once a pending one closes: a screen with its own
    /// keymap (the interactive rebase todo) imposes it for as long as it is up.
    pub(crate) fn base_menu(&self) -> Option<Menu> {
        self.screen().menu.or_else(|| root_menu(&self.state.config))
    }

    pub fn inhibit_close_menu(&mut self) {
        self.state.inhibit_close_menu = true;
    }

    pub(crate) fn screen_mut(&mut self) -> &mut Screen {
        self.state.screens.last_mut().expect("No screen")
    }

    pub(crate) fn screen(&self) -> &Screen {
        self.state.screens.last().expect("No screen")
    }

    /// Displays an `Info` message to the CmdLog.
    pub fn display_info<S: Into<Cow<'static, str>>>(&mut self, message: S) {
        self.state
            .current_cmd_log
            .push(CmdLogEntry::Info(message.into().into_owned()));
    }

    /// Displays an `Error` message to the CmdLog.
    pub fn display_error<S: Into<Cow<'static, str>>>(&mut self, message: S) {
        self.state
            .current_cmd_log
            .push(CmdLogEntry::Error(message.into().into_owned()));
    }

    /// Runs a `Command` and handles its output.
    /// Will block awaiting its completion.
    pub fn run_cmd(&mut self, term: &mut Term, input: &[u8], cmd: Command) -> Res<()> {
        self.run_cmd_async(term, input, cmd)?;
        self.await_pending_cmd()?;
        self.handle_pending_cmd()?;
        Ok(())
    }

    /// Runs a `Command` and handles its output asynchronously (if async commands are enabled).
    /// Will return `Ok(())` if one is already running.
    pub fn run_cmd_async(&mut self, term: &mut Term, input: &[u8], mut cmd: Command) -> Res<()> {
        cmd.env("CLICOLOR_FORCE", "1"); // No guarantee, but modern tools seem to implement this

        if self.state.pending_cmd.is_some() {
            return Err(Error::CmdAlreadyRunning);
        }

        cmd.current_dir(self.state.repo.workdir().expect("No workdir"));

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let log_entry = self.state.current_cmd_log.push_cmd(&cmd);

        ui::ui(term, &mut self.state)?;

        let mut child = cmd.spawn().map_err(Error::SpawnCmd)?;

        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input)
            .map_err(Error::Term)?;

        self.state.pending_cmd = Some((child, log_entry));

        if !self.state.enable_async_cmds {
            self.await_pending_cmd()?;
        }

        Ok(())
    }

    fn await_pending_cmd(&mut self) -> Res<()> {
        if let Some((child, _)) = &mut self.state.pending_cmd {
            child.wait().map_err(Error::CouldntAwaitCmd)?;
        }
        Ok(())
    }

    /// Handles any pending_cmd in State without blocking. Returns `true` if a cmd was handled.
    fn handle_pending_cmd(&mut self) -> Res<()> {
        let Some((ref mut child, ref mut log_rwlock)) = self.state.pending_cmd else {
            return Ok(());
        };

        let Some(status) = child.try_wait().map_err(Error::CouldntAwaitCmd)? else {
            return Ok(());
        };

        log::debug!("pending cmd finished with {status:?}");

        let result = write_child_output_to_log(log_rwlock, child, status);
        self.state.pending_cmd = None;
        self.update_screens()?;
        result?;

        Ok(())
    }

    pub fn run_cmd_interactive(&mut self, term: &mut Term, mut cmd: Command) -> Res<()> {
        cmd.env("CLICOLOR_FORCE", "1"); // No guarantee, but modern tools seem to implement this

        if self.state.pending_cmd.is_some() {
            return Err(Error::CmdAlreadyRunning);
        }

        cmd.current_dir(self.state.repo.workdir().ok_or(Error::NoRepoWorkdir)?);

        self.state
            .current_cmd_log
            .push_cmd_with_output(&cmd, "\n".into());
        self.redraw_now(term)?;

        eprint!("\r");

        // Redirect stderr so we can capture it via `Child::wait_with_output()`
        cmd.stderr(Stdio::piped());

        term.reset_term_stay_on_alt_screeen(&self.state.config)
            .map_err(Error::Term)?;

        let mut child = cmd.spawn().map_err(Error::SpawnCmd)?;

        // Drop stdin as `Child::wait_with_output` would
        drop(child.stdin.take());

        let mut stderr = Vec::new();

        tee(
            child.stderr.as_mut(),
            &mut [&mut std::io::stderr(), &mut stderr],
        )
        .map_err(Error::Term)?;

        let status = child.wait().map_err(Error::CouldntAwaitCmd)?;

        term.setup_term(&self.state.config).map_err(Error::Term)?;

        let out_utf8 = String::from_utf8(strip_ansi_escapes::strip(stderr.clone()))
            .expect("Error turning command output to String")
            .into();

        self.state.current_cmd_log.clear();
        self.state
            .current_cmd_log
            .push_cmd_with_output(&cmd, out_utf8);

        term.clear().map_err(Error::Term)?;
        self.update_screens()?;

        if !status.success() {
            return Err(Error::CmdBadExit(
                format!(
                    "{} {}",
                    cmd.get_program().to_string_lossy(),
                    cmd.get_args()
                        .map(|arg| arg.to_string_lossy())
                        .collect::<String>()
                ),
                status.code(),
            ));
        }

        Ok(())
    }

    pub fn hide_menu(&mut self) {
        if let Some(ref mut menu) = self.state.pending_menu {
            menu.is_hidden = true;
        }
    }

    fn unhide_menu(&mut self) {
        if let Some(ref mut menu) = self.state.pending_menu {
            menu.is_hidden = false;
        }
    }

    pub(crate) fn selected_rev(&self) -> Option<Rev> {
        self.screen().get_selected_item().data.rev()
    }

    pub(crate) fn prompt(&mut self, term: &mut Term, params: &PromptParams) -> Res<String> {
        let default = (params.create_default_value)(self);

        // A prompt with nothing to say says nothing: the key that opened it
        // already said what is being answered. Prefilled, the default is the
        // answer being edited rather than the one an empty answer falls back
        // to — it is on the line, so naming it again says nothing either.
        let prompt_text = match (params.prompt, &default) {
            ("", _) => Cow::Borrowed(""),
            (prompt, _) if params.bare => Cow::Borrowed(prompt),
            (prompt, Some(default)) if !params.prefill => {
                format!("{prompt} (default {default}):").into()
            }
            (prompt, _) => format!("{prompt}:").into(),
        };

        if params.hide_menu {
            self.hide_menu();
        }

        self.state.prompt.set(prompt::PromptData {
            prompt_text,
            bare: params.bare,
        });

        if params.prefill
            && let Some(default) = default
        {
            self.state.prompt.state.value = default;
            self.state.prompt.state.move_to(usize::MAX);
        }
        self.state.prompt.start_history(params.history)?;

        let result = self.handle_prompt(term, params);

        self.unhide_menu();
        self.state.prompt.reset();

        result
    }

    fn handle_prompt(&mut self, term: &mut Term, params: &PromptParams) -> Res<String> {
        self.redraw_now(term)?;

        loop {
            let event = term.read_event()?;
            self.handle_event(term, event)?;

            if self.state.prompt.state.status == text_input::Status::Done {
                let value = get_prompt_result(params, self)?;
                if let Some(kind) = params.history {
                    self.state.prompt.remember(kind, &value)?;
                }
                return Ok(value);
            } else if self.state.prompt.state.status == text_input::Status::Aborted {
                return Err(Error::PromptAborted);
            }

            self.redraw_now(term)?;
        }
    }

    pub fn confirm(&mut self, term: &mut Term, prompt: &'static str) -> Res<()> {
        self.hide_menu();
        self.state.prompt.set(prompt::PromptData {
            prompt_text: prompt.into(),
            bare: false,
        });

        let result = self.handle_confirm(term);

        self.unhide_menu();
        self.state.prompt.reset();

        result
    }

    fn handle_confirm(&mut self, term: &mut Term) -> Res<()> {
        self.redraw_now(term)?;
        loop {
            let event = term.read_event()?;
            self.handle_event(term, event)?;

            match self.state.prompt.state.value.as_str() {
                "y" => {
                    return Ok(());
                }
                "" => (),
                _ => {
                    return Err(Error::PromptAborted);
                }
            }

            self.redraw_now(term)?;
        }
    }

    /// Show a picker and wait for user to select an item or cancel.
    ///
    /// Returns:
    /// - `Ok(Some(data))` - User selected an item
    /// - `Ok(None)` - User cancelled (Esc or Ctrl-C)
    /// - `Err(e)` - An error occurred
    ///
    /// # Example
    /// ```ignore
    /// let items = vec![
    ///     PickerItem::new("main", PickerData::Item("main".to_string())),
    ///     PickerItem::new("develop", PickerData::Item("develop".to_string())),
    /// ];
    /// let picker = PickerState::new("Select branch", items, false);
    ///
    /// match app.pick(term, picker)? {
    ///     Some(PickerData::Item(name)) => {
    ///         // User selected a branch
    ///         println!("Selected: {}", name);
    ///     }
    ///     Some(PickerData::CustomInput(_)) => {
    ///         // Should not happen when allow_custom_input is false
    ///     }
    ///     None => {
    ///         // User cancelled
    ///         println!("Cancelled");
    ///     }
    /// }
    /// ```
    pub fn pick(&mut self, term: &mut Term, picker_state: PickerState) -> Res<Option<PickerData>> {
        Ok(self
            .pick_or_set_key(term, picker_state)?
            .map(|picked| match picked {
                Picked::Item(data) => data,
                Picked::KeySet(..) => unreachable!("only a picker assigning keys sets them"),
            }))
    }

    /// [`App::pick`], for a picker whose items can be given keys.
    pub(crate) fn pick_or_set_key(
        &mut self,
        term: &mut Term,
        picker_state: PickerState,
    ) -> Res<Option<Picked>> {
        self.state.picker = Some(picker_state);
        let result = self.handle_picker(term, &mut |_, _| Ok(()));

        self.state.picker = None;

        result
    }

    /// [`App::pick`], calling `preview` with each item the cursor stops on.
    pub(crate) fn pick_previewing(
        &mut self,
        term: &mut Term,
        picker_state: PickerState,
        preview: &mut dyn FnMut(&mut App, &PickerData) -> Res<()>,
    ) -> Res<Option<PickerData>> {
        self.state.picker = Some(picker_state);
        let result = self.handle_picker(term, preview);

        self.state.picker = None;

        Ok(result?.map(|picked| match picked {
            Picked::Item(data) => data,
            Picked::KeySet(..) => unreachable!("only a picker assigning keys sets them"),
        }))
    }

    /// Choose a commit by moving around a log view: the screen is pushed with
    /// its own keymap, and the loop runs until `select_commit` names one or the
    /// screen is closed.
    pub fn pick_commit(&mut self, term: &mut Term) -> Res<Option<String>> {
        let params = self.render_params(self.screen().size);
        let selected = match &self.screen().get_selected_item().data {
            crate::item_data::ItemData::Commit { oid, .. } => Some(oid.clone()),
            _ => None,
        };
        // The selected commit has to be found among the rows, so they are all
        // drawn first.
        let params = RenderParams {
            streaming: params.streaming && selected.is_none(),
            ..params
        };
        self.state.screens.push(screen::pager::log(
            Arc::clone(&self.state.config),
            Rc::clone(&self.state.repo),
            params,
            COMMIT_PICK_LIMIT,
            None,
            None,
        )?);
        self.screen_mut().menu = Some(Menu::CommitPicker);
        self.close_menu();

        // Start on the commit that was already selected, if there was one.
        if let Some(selected) = selected {
            self.screen_mut().select_matching(|data| {
                matches!(data, crate::item_data::ItemData::Commit { oid, .. } if *oid == selected)
            });
        }

        let depth = self.state.screens.len();
        self.redraw_now(term)?;

        loop {
            let event = term.read_event()?;
            self.handle_event(term, event)?;

            if let Some(oid) = self.state.picked_commit.take() {
                self.state.screens.truncate(depth - 1);
                self.close_menu();
                return Ok(Some(oid));
            }
            if self.state.screens.len() < depth {
                return Ok(None); // The screen was closed: nothing picked.
            }

            self.redraw_now(term)?;
        }
    }

    fn handle_picker(
        &mut self,
        term: &mut Term,
        preview: &mut dyn FnMut(&mut App, &PickerData) -> Res<()>,
    ) -> Res<Option<Picked>> {
        self.redraw_now(term)?;
        let selected = |app: &App| {
            app.state
                .picker
                .as_ref()
                .and_then(|picker| picker.selected())
                .map(|item| item.data.clone())
        };
        let mut previewed = selected(self);

        loop {
            let event = term.read_event()?;
            self.handle_event(term, event)?;

            if let Some(ref picker) = self.state.picker {
                let selected = || picker.selected().map(|item| item.data.clone());
                match *picker.status() {
                    PickerStatus::Done => return Ok(selected().map(Picked::Item)),
                    PickerStatus::KeySet(key) => {
                        return Ok(selected().map(|data| Picked::KeySet(key, data)));
                    }
                    // User cancelled - this is not an error
                    PickerStatus::Cancelled => return Ok(None),
                    PickerStatus::Active => (),
                }
            }

            // Only once the keys that came together are handled, so that an
            // arrow held down does not preview every item it passes.
            let now = selected(self);
            if now != previewed && !matches!(term.poll_event(Duration::ZERO), Ok(true)) {
                if let Some(data) = &now {
                    preview(self, data)?;
                }
                previewed = now;
            }

            self.redraw_now(term)?;
        }
    }

    /// The feature that `key` toggles in the picker's one row, if any.
    fn feature_with_key_in_one_row(&self, key: event::KeyEvent) -> Option<String> {
        let picker = self.state.picker.as_ref()?;
        let quit = &self.state.config.picker_bindings.quit;
        match (key.code, key.modifiers) {
            (event::KeyCode::Char(c), event::KeyModifiers::NONE)
                if picker.is_one_row()
                    && !c.is_ascii_digit()
                    && !quit.iter().any(|b| b == &vec![(key.modifiers, key.code)]) =>
            {
                self.feature_with_key(c)
            }
            _ => None,
        }
    }

    /// The feature that `key` toggles, if any.
    fn feature_with_key(&self, key: char) -> Option<String> {
        self.state
            .feature_keys
            .iter()
            .find(|(_, held)| **held == key)
            .map(|(feature, _)| feature.clone())
    }

    fn handle_picker_input(&mut self, key: event::KeyEvent) -> Res<()> {
        if key.kind != event::KeyEventKind::Press {
            return Ok(());
        }

        // Nothing is typed in theme mode's one row, so a feature key toggles its
        // feature there as at the top level, and themes can be judged with it on
        // and off. A digit there selects a theme, and `picker.quit` quits.
        if let Some(feature) = self.feature_with_key_in_one_row(key) {
            return crate::ops::editor::toggle_feature(self, &feature);
        }

        if let Some(ref mut picker) = self.state.picker {
            let bindings = &self.state.config.picker_bindings;
            let key_combo = vec![(key.modifiers, key.code)];
            let typed = match (key.code, key.modifiers) {
                (
                    event::KeyCode::Char(c),
                    event::KeyModifiers::NONE | event::KeyModifiers::SHIFT,
                ) => Some(c),
                _ => None,
            };

            if bindings.next.iter().any(|b| b == &key_combo) {
                picker.next();
            } else if bindings.previous.iter().any(|b| b == &key_combo) {
                picker.previous();
            } else if bindings.done.iter().any(|b| b == &key_combo) {
                picker.done();
            } else if bindings.cancel.iter().any(|b| b == &key_combo) {
                picker.cancel();
            } else if bindings.toggle_list.iter().any(|b| b == &key_combo) {
                picker.toggle_one_row();
            } else if picker.is_one_row() {
                // Nothing is typed here, so a letter can quit and a digit select.
                if bindings.quit.iter().any(|b| b == &key_combo) {
                    picker.cancel();
                } else if let Some(digit) = typed.and_then(|c| c.to_digit(10)) {
                    picker.select_number(digit as usize);
                }
            } else if picker.assigns_keys() {
                // A letter sets a key here, but not the quit letter, which is
                // bound at the top level and so could not be set anyway.
                if bindings.quit.iter().any(|b| b == &key_combo) {
                    picker.cancel();
                } else if let Some(c) = typed {
                    picker.set_key(c);
                }
            } else {
                // Text input - delegate to text state
                picker.input_state.handle_key_event(key);
                picker.update_filter();
            }
        }
        Ok(())
    }
}

/// `features`, then `theme`: last, so that it has its way over a feature that
/// sets the same keys, as delta resolves a conflict between features in favour
/// of the later one.
fn overlay(features: &[String], theme: Option<&str>) -> Rc<[String]> {
    features
        .iter()
        .cloned()
        .chain(theme.map(str::to_owned))
        .collect()
}

fn get_prompt_result(params: &PromptParams, app: &mut App) -> Res<String> {
    let input = app.state.prompt.state.value.as_str();
    if params.prefill {
        return Ok(input.to_owned());
    }

    let default_value = (params.create_default_value)(app);

    let value = match (input, &default_value) {
        ("", None) => "",
        ("", Some(selected)) => selected,
        (value, _) => value,
    };

    Ok(value.to_string())
}

fn tee(maybe_input: Option<&mut impl Read>, outputs: &mut [&mut dyn Write]) -> std::io::Result<()> {
    let Some(input) = maybe_input else {
        return Ok(());
    };

    let mut buf = [0u8; 1024];

    loop {
        let num_read = input.read(&mut buf)?;
        if num_read == 0 {
            break;
        }

        let buf = &buf[..num_read];
        for output in &mut *outputs {
            output.write_all(buf)?;
        }
    }

    Ok(())
}

fn rebase_todo_screen(
    config: &Arc<Config>,
    repo: &Rc<Repository>,
    params: RenderParams,
    todo: RebaseTodo,
) -> Res<Screen> {
    screen::rebase_todo::create(
        Arc::clone(config),
        Rc::clone(repo),
        params,
        Rc::new(RefCell::new(todo)),
    )
}

/// How far back [`App::pick_commit`] lists commits.
const COMMIT_PICK_LIMIT: usize = 256;

/// How long after the last resize the screens are rendered again at the new width. Tests, which
/// have no async commands, render as soon as no event is waiting.
const RESIZE_RENDER_DELAY: Duration = Duration::from_millis(150);

pub(crate) fn root_menu(config: &Config) -> Option<Menu> {
    if config.general.always_show_help.enabled {
        Some(Menu::Help)
    } else {
        None
    }
}

fn write_child_output_to_log(
    log_rwlock: &mut Arc<RwLock<CmdLogEntry>>,
    child: &mut Child,
    status: std::process::ExitStatus,
) -> Res<()> {
    let mut log = log_rwlock.write().unwrap();

    let CmdLogEntry::Cmd { args, out: out_log } = log.deref_mut() else {
        unreachable!("pending_cmd is always CmdLogEntry::Cmd variant");
    };

    drop(child.stdin.take());

    let mut out_bytes = vec![];
    log::debug!("Reading stderr");

    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut out_bytes)
        .map_err(Error::CouldntReadCmdOutput)?;

    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut out_bytes)
        .map_err(Error::CouldntReadCmdOutput)?;

    let out_string = String::from_utf8_lossy(&out_bytes).to_string();
    *out_log = Some(out_string.into());

    if !status.success() {
        return Err(Error::CmdBadExit(args.to_string(), status.code()));
    }

    Ok(())
}

type DefaultFn = Box<dyn Fn(&App) -> Option<String>>;

pub(crate) struct PromptParams {
    pub prompt: &'static str,
    pub create_default_value: DefaultFn,
    pub hide_menu: bool,
    /// Put the default value in the prompt for the user to edit, rather than
    /// keeping it as what an empty answer means.
    pub prefill: bool,
    pub history: Option<prompt::HistoryKind>,
    /// Draw `prompt` alone in front of the line typed on, as less and vim draw
    /// `/` for a search, rather than as a question.
    pub bare: bool,
}

impl Default for PromptParams {
    fn default() -> Self {
        Self {
            prompt: "",
            create_default_value: Box::new(|_| None),
            hide_menu: true,
            prefill: false,
            history: None,
            bare: false,
        }
    }
}
