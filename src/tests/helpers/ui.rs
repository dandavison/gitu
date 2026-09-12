use crate::{
    app::App,
    cli::Args,
    config::{self, Config},
    error::Error,
    key_parser::parse_test_keys,
    screen::pager::Paged,
    term::{Term, TermBackend},
    tests::helpers::RepoTestContext,
};
use crossterm::event::{Event, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use git2::Repository;
use ratatui::{Terminal, backend::TestBackend, layout::Size};
use regex::Regex;
use std::{path::PathBuf, rc::Rc, sync::Arc, time::Duration};
use unicode_segmentation::UnicodeSegmentation;

use self::buffer::TestBuffer;

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
    pub size: Size,
    config: Arc<Config>,
}

#[macro_export]
macro_rules! setup_clone {
    () => {{ TestContext::setup_clone(function_name!()) }};
}

impl TestContext {
    pub fn setup_clone(test_name: &str) -> Self {
        let size = Size::new(80, 20);
        let term = Terminal::new(TermBackend::Test {
            backend: TestBackend::new(size.width, size.height),
            events: vec![],
            draws: vec![],
        })
        .unwrap();
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
        let text = crate::tests::helpers::run(&self.dir, cmd);
        self.init_app_paged(Paged {
            text,
            git_argv: Some(cmd.iter().map(|arg| (*arg).to_owned()).collect()),
        })
    }

    /// Start gitu on piped text no command of git's is known to have produced.
    pub fn init_app_with_patch(&mut self, patch: String) -> App {
        self.init_app_paged(Paged {
            text: patch,
            git_argv: None,
        })
    }

    fn init_app_paged(&mut self, paged: Paged) -> App {
        self.init_app_inner(
            self.dir.to_path_buf(),
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
        let TermBackend::Test { events, .. } = self.term.backend_mut() else {
            unreachable!();
        };

        events.extend(new_events.into_iter().rev());

        let result = app.run(&mut self.term, Duration::ZERO);
        assert!(app.state.quit || matches!(result, Err(Error::NoMoreEvents)));
    }

    pub fn redact_buffer(&self) -> String {
        let TermBackend::Test { backend, .. } = self.term.backend() else {
            unreachable!();
        };
        let mut debug_output = format!("{:?}", TestBuffer(backend.buffer()));

        redact(&mut debug_output, "From file://(.*)\n");
        redact(&mut debug_output, "To file://(/.*)\n");

        debug_output
    }

    pub fn physical_screen(&self) -> String {
        let TermBackend::Test { draws, .. } = self.term.backend() else {
            unreachable!();
        };
        let mut physical = blank_screen(self.size);
        for draw in draws {
            for (x, y, symbol) in draw {
                draw_symbol(&mut physical, *x, *y, symbol);
            }
        }
        screen_text(&physical)
    }
}

fn blank_screen(size: Size) -> Vec<Vec<String>> {
    vec![vec![" ".to_string(); size.width as usize]; size.height as usize]
}

fn draw_symbol(screen: &mut [Vec<String>], x: u16, y: u16, symbol: &str) {
    let Some(row) = screen.get_mut(y as usize) else {
        return;
    };
    for (column, grapheme) in crate::ui::display_text(symbol).graphemes(true).enumerate() {
        if let Some(cell) = row.get_mut(x as usize + column) {
            *cell = grapheme.to_string();
        }
    }
}

fn screen_text(screen: &[Vec<String>]) -> String {
    screen
        .iter()
        .map(|row| row.concat())
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact(debug_output: &mut String, regex: &str) {
    let re = Regex::new(regex).unwrap();
    if let Some(caps) = re.captures(debug_output) {
        let c = caps.get(1).unwrap();
        debug_output.replace_range(c.range(), &" ".repeat(c.len()));
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
