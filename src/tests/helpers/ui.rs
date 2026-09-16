use crate::style::Modifier;
use crate::{
    app::App,
    capped_input::Capped,
    cli::Args,
    config::{self, Config, TEST_SEARCH_HIGHLIGHT_BG},
    error::Error,
    key_parser::parse_test_keys,
    screen::pager::Paged,
    term::{Term, TermBackend, TestBuffer},
    tests::helpers::RepoTestContext,
};
use crossterm::event::{Event, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use git2::Repository;
use regex::Regex;
use std::{path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use self::buffer::DebugBuffer;

mod buffer;

#[macro_export]
macro_rules! snapshot {
    ($ctx:expr, $keys:expr) => {{
        let mut ctx = $ctx;
        let mut state = ctx.init_app();
        ctx.update(&mut state, keys($keys));
        insta::assert_snapshot!(ctx.redact_buffer());
        state
    }};
}

pub struct TestContext {
    pub term: Term,
    pub dir: PathBuf,
    pub remote_dir: PathBuf,
    pub size: (u16, u16),
    config: Arc<Config>,
}

#[macro_export]
macro_rules! setup_clone {
    () => {{ TestContext::setup_clone(function_name!()) }};
}

#[macro_export]
macro_rules! setup_clone_wide {
    () => {{ TestContext::setup_clone_wide(function_name!()) }};
}

impl TestContext {
    pub fn setup_clone(test_name: &str) -> Self {
        Self::setup_clone_sized(test_name, (80, 20))
    }

    /// For tests whose output carries the remote's path. At 80 columns that
    /// wraps on a longer checkout, which shifts every row of the frame and
    /// leaves the path visible past what [`Self::redact_buffer`] blanks.
    pub fn setup_clone_wide(test_name: &str) -> Self {
        Self::setup_clone_sized(test_name, (120, 20))
    }

    fn setup_clone_sized(test_name: &str, size: (u16, u16)) -> Self {
        let term = TermBackend::Test {
            buffer: TestBuffer::new(size.0, size.1),
            events: vec![],
        };
        let repo_ctx = RepoTestContext::setup_clone(test_name);
        Self {
            term,
            dir: repo_ctx.dir,
            remote_dir: repo_ctx.remote_dir,
            size,
            config: Arc::new(config::init_test_config().unwrap()),
        }
    }

    pub fn config(&mut self) -> &mut Config {
        Arc::get_mut(&mut self.config).unwrap()
    }

    pub fn init_app(&mut self) -> App {
        self.init_app_at_path(self.dir.to_path_buf())
    }

    pub fn init_app_at_path(&mut self, path: PathBuf) -> App {
        self.init_app_with_args(path, Args::default())
    }

    /// Start gitu as git's pager would (`[pager] diff = gitu --pager`), on the
    /// output of `cmd` and knowing that `cmd` is what produced it.
    pub fn init_app_as_pager_of(&mut self, cmd: &[&str]) -> App {
        self.init_app_as_pager_of_at(self.dir.clone(), cmd)
    }

    pub fn init_app_as_pager_of_at(&mut self, path: PathBuf, cmd: &[&str]) -> App {
        let text = crate::tests::helpers::run(&path, cmd);
        let paged = self.piped(
            text,
            Some(cmd.iter().map(|arg| (*arg).to_owned()).collect()),
        );
        self.init_app_paged_at(path, paged)
    }

    /// Start gitu on piped text no command of git's is known to have produced.
    pub fn init_app_with_patch(&mut self, patch: String) -> App {
        let paged = self.piped(patch, None);
        self.init_app_paged(paged)
    }

    /// `text` as gitu reads it from a pipe: up to the byte limit in config.
    fn piped(&self, text: String, git_argv: Option<Vec<String>>) -> Paged {
        let input = Capped::read(text.as_bytes(), self.config.general.max_input_bytes).unwrap();
        Paged {
            text: input.text,
            truncated: input.truncated,
            git_argv,
        }
    }

    fn init_app_paged(&mut self, paged: Paged) -> App {
        self.init_app_paged_at(self.dir.clone(), paged)
    }

    fn init_app_paged_at(&mut self, path: PathBuf, paged: Paged) -> App {
        self.init_app_inner(
            path,
            Args {
                pager: true,
                ..Default::default()
            },
            Some(paged),
        )
    }

    /// Start gitu as one of its subcommands would (`gitu sequence-editor …`).
    pub fn init_app_with_args(&mut self, path: PathBuf, args: Args) -> App {
        self.init_app_inner(path, args, None)
    }

    fn init_app_inner(&mut self, path: PathBuf, args: Args, paged: Option<Paged>) -> App {
        let mut app = App::create(
            Rc::new(Repository::open(path).unwrap()),
            self.size,
            &args,
            Arc::clone(&self.config),
            false,
            paged,
        )
        .unwrap();

        app.redraw_now(&mut self.term).unwrap();
        app
    }

    pub fn update(&mut self, app: &mut App, new_events: Vec<Event>) {
        let TermBackend::Test { events, .. } = &mut self.term else {
            unreachable!();
        };

        events.extend(new_events.into_iter().rev());

        let result = app.run(&mut self.term, Duration::ZERO);
        assert!(app.state.quit || matches!(result, Err(Error::NoMoreEvents)));
    }

    /// The text of every search match marked on screen, in reading order.
    pub fn highlighted(&self) -> Vec<String> {
        let TermBackend::Test { buffer, .. } = &self.term else {
            unreachable!();
        };

        let mut marked: Vec<String> = vec![];
        let mut last = None;

        for (i, cell) in buffer.cells.iter().enumerate() {
            if cell.bg != TEST_SEARCH_HIGHLIGHT_BG {
                continue;
            }

            match last {
                Some(before) if before + 1 == i && i % buffer.width as usize != 0 => {
                    marked.last_mut().unwrap().push_str(&cell.symbol)
                }
                _ => marked.push(cell.symbol.clone()),
            }

            last = Some(i);
        }

        marked
    }

    /// The line being typed, with `|` where the cursor is drawn: what the user
    /// can see of where their next character will land.
    pub fn prompt_line(&self) -> String {
        let TermBackend::Test { buffer, .. } = &self.term else {
            unreachable!();
        };
        let row = buffer.height - 1;

        let mut line = String::new();
        for column in 0..buffer.width {
            let cell = &buffer.cells[row as usize * buffer.width as usize + column as usize];
            // The block standing on its own is the cursor; a reversed cell is
            // the cursor standing on a character.
            if cell.symbol == "\u{2588}" {
                line.push('|');
                continue;
            }
            if cell.modifier.contains(Modifier::REVERSED) {
                line.push('|');
            }
            line.push_str(&cell.symbol);
        }

        line.trim_end().to_owned()
    }

    pub fn redact_buffer(&self) -> String {
        let TermBackend::Test { buffer, .. } = &self.term else {
            unreachable!();
        };
        let mut debug_output = format!("{:?}", DebugBuffer(buffer));

        redact(&mut debug_output, "From file://(.*)\n");
        redact(&mut debug_output, "To file://(/.*)\n");
        // The fixtures' commit dates are fixed, so their age grows as time
        // passes. Only its width, which the layout depends on, is kept.
        redact_all(&mut debug_output, r"(\d+[mhdwMy]) \|");

        debug_output
    }

    /// Every row as it stands on the terminal, links and all.
    pub fn physical_screen(&self) -> String {
        let TermBackend::Test { buffer, .. } = &self.term else {
            unreachable!();
        };

        buffer
            .cells
            .chunks(buffer.width as usize)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn redact(debug_output: &mut String, regex: &str) {
    let re = Regex::new(regex).unwrap();
    if let Some(caps) = re.captures(debug_output) {
        let c = caps.get(1).unwrap();
        debug_output.replace_range(c.range(), &" ".repeat(c.len()));
    }
}

/// Replaces every capture with `_`, so that what was redacted stays visible.
fn redact_all(debug_output: &mut String, regex: &str) {
    let re = Regex::new(regex).unwrap();
    let ranges = re
        .captures_iter(debug_output)
        .map(|caps| caps.get(1).unwrap().range())
        .collect::<Vec<_>>();

    for range in ranges.into_iter().rev() {
        let placeholder = "_".repeat(range.len());
        debug_output.replace_range(range, &placeholder);
    }
}

pub fn keys(input: &str) -> Vec<Event> {
    let ("", keys) = parse_test_keys(input).unwrap() else {
        unreachable!();
    };

    keys.into_iter()
        .map(|(mods, key)| Event::Key(KeyEvent::new(key, mods)))
        .collect()
}

pub fn mouse_event(x: u16, y: u16, mouse_button: MouseButton) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(mouse_button),
        column: x,
        row: y.saturating_sub(1),
        modifiers: KeyModifiers::NONE,
    })
}

pub fn mouse_scroll_event(x: u16, y: u16, scroll_up: bool) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind: if scroll_up {
            MouseEventKind::ScrollUp
        } else {
            MouseEventKind::ScrollDown
        },
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}
