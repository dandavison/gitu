//! A command's output piped through the renderer as it is written, as a shell
//! pipeline would: what the renderer has drawn can be shown while the rest of
//! it is still being written and drawn.

use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

/// Where the text comes from: a command gitu runs, or text already in hand.
pub(crate) enum Input {
    Command(Command),
    Text(String),
}

/// A pipeline from an [`Input`] through a renderer, read on threads of its own.
/// Dropping it stops reading the input; what has been read is still drawn.
pub(crate) struct Stream {
    shared: Arc<Shared>,
}

/// What a [`Stream`] has produced so far.
pub(crate) struct Snapshot {
    /// What the input wrote. While it is still being written, or once reading
    /// stopped at the limit, it is cut back to what is whole: a hunk missing
    /// its last lines is one git will not apply.
    pub text: String,
    /// Whether reading stopped at the limit rather than at the end.
    pub truncated: bool,
    /// What the renderer drew, up to its last whole row. `None` without a
    /// renderer, or when it could not draw.
    pub rendered: Option<String>,
    /// Whether the input and the renderer have both finished.
    pub done: bool,
    /// What git said when it exited unsuccessfully having written nothing.
    pub refused: Option<String>,
    /// Whether the command exited unsuccessfully, whatever it wrote.
    pub failed: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    input: Vec<u8>,
    input_rows: usize,
    truncated: bool,
    input_done: bool,
    refused: Option<String>,
    failed: bool,
    has_renderer: bool,
    rendered: Option<Vec<u8>>,
    rendered_rows: usize,
    rendered_done: bool,
    abandoned: bool,
}

impl State {
    fn done(&self) -> bool {
        self.input_done && self.rendered_done
    }

    /// Rows there are to show: the renderer's, or the input's own without one.
    fn rows(&self) -> usize {
        if self.has_renderer {
            self.rendered_rows
        } else {
            self.input_rows
        }
    }
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().unwrap());
        self.changed.notify_all();
    }
}

impl Stream {
    /// Start `input` writing into `renderer`, reading at most `limit` bytes of
    /// it. Without a renderer, or with one that cannot be started, the input is
    /// only read.
    pub(crate) fn start(input: Input, renderer: Option<Command>, limit: u64) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });

        let (source, git): (Box<dyn Read + Send>, Option<Child>) = match input {
            Input::Text(text) => (Box::new(io::Cursor::new(text.into_bytes())), None),
            Input::Command(mut command) => {
                let mut child = command
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                let stdout = child.stdout.take().expect("stdout was piped");
                (Box::new(stdout), Some(child))
            }
        };

        let mut renderer = renderer.and_then(|mut command| {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .inspect_err(|e| log::warn!("renderer failed to spawn: {e}"))
                .ok()
        });
        let sink = renderer.as_mut().and_then(|child| child.stdin.take());

        match renderer {
            Some(mut child) => {
                let stdout = child.stdout.take().expect("stdout was piped");
                shared.update(|state| {
                    state.has_renderer = true;
                    state.rendered = Some(Vec::new());
                });
                let shared = Arc::clone(&shared);
                thread::spawn(move || draw(&shared, stdout, child));
            }
            None => shared.update(|state| state.rendered_done = true),
        }

        let reader = Arc::clone(&shared);
        thread::spawn(move || read(&reader, source, sink, git, limit));

        Ok(Self { shared })
    }

    /// Wait until there are `rows` rows to show, or everything is done.
    pub(crate) fn wait_for(&self, rows: usize) {
        let mut state = self.shared.state.lock().unwrap();
        while !state.done() && state.rows() < rows {
            state = self.shared.changed.wait(state).unwrap();
        }
    }

    pub(crate) fn is_done(&self) -> bool {
        self.shared.state.lock().unwrap().done()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.shared.state.lock().unwrap();
        let done = state.done();

        let text = String::from_utf8_lossy(&state.input);
        let text = if !state.input_done || state.truncated {
            crate::capped_input::whole(&text).to_owned()
        } else {
            text.into_owned()
        };

        let rendered = state.rendered.as_ref().map(|rendered| {
            let rows = match rendered.iter().rposition(|&byte| byte == b'\n') {
                _ if done => rendered.as_slice(),
                Some(end) => &rendered[..=end],
                None => &[],
            };
            String::from_utf8_lossy(rows).into_owned()
        });

        Snapshot {
            text,
            truncated: state.truncated,
            rendered,
            done,
            refused: state.refused.clone(),
            failed: state.failed,
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.update(|state| state.abandoned = true);
    }
}

/// Read `source` into the shared state and on into the renderer, up to one
/// byte past `limit` (so that stopping there is distinguishable from input
/// exactly that long). A command still writing past the limit is killed.
fn read(
    shared: &Shared,
    mut source: Box<dyn Read + Send>,
    mut sink: Option<ChildStdin>,
    git: Option<Child>,
    limit: u64,
) {
    // Read stderr on its own thread: a command filling that pipe while gitu is
    // reading stdout would otherwise deadlock.
    let mut git = git;
    let complaint = git
        .as_mut()
        .and_then(|child| child.stderr.take())
        .map(|mut pipe| {
            thread::spawn(move || {
                let mut said = String::new();
                let _ = pipe.read_to_string(&mut said);
                said
            })
        });

    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0;
    let mut stopped = false;
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let keep = read.min(
            limit
                .saturating_add(1)
                .saturating_sub(total)
                .try_into()
                .unwrap_or(usize::MAX),
        );
        total += keep as u64;
        let chunk = &buffer[..keep];

        if let Some(stdin) = &mut sink
            && stdin.write_all(chunk).is_err()
        {
            sink = None;
        }

        let mut abandoned = false;
        shared.update(|state| {
            state.input.extend_from_slice(chunk);
            state.input_rows += chunk.iter().filter(|&&byte| byte == b'\n').count();
            abandoned = state.abandoned;
        });
        if total > limit || abandoned {
            stopped = total > limit;
            break;
        }
    }
    drop(sink);

    let mut refused = None;
    let mut failed = false;
    if let Some(mut child) = git {
        if stopped {
            // The rest of it is not going to be read, so it is not left to be
            // written either.
            let _ = child.kill();
        }
        let succeeded = child.wait().is_ok_and(|status| status.success());
        failed = !succeeded;
        let said = complaint.and_then(|thread| thread.join().ok());
        // A command the user can edit is a command the user can get wrong, and
        // an empty screen does not say what git thought of it. (`git diff
        // --exit-code` answers with a status too, and a killed command has one
        // as well, so a command that answered at all is taken to have worked.)
        if !succeeded && total == 0 {
            refused = said.map(|said| said.lines().next().unwrap_or_default().to_owned());
        }
    }

    shared.update(|state| {
        state.truncated = stopped;
        state.refused = refused;
        state.failed = failed;
        state.input_done = true;
    });
}

/// Read the renderer's output into the shared state as it draws it.
fn draw(shared: &Shared, mut stdout: ChildStdout, mut child: Child) {
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = match stdout.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let chunk = &buffer[..read];
        shared.update(|state| {
            if let Some(rendered) = &mut state.rendered {
                rendered.extend_from_slice(chunk);
            }
            state.rendered_rows += chunk.iter().filter(|&&byte| byte == b'\n').count();
        });
    }

    let succeeded = child.wait().is_ok_and(|status| status.success());
    if !succeeded {
        log::warn!("renderer exited unsuccessfully");
    }
    shared.update(|state| {
        if !succeeded {
            state.rendered = None;
        }
        state.rendered_done = true;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const PATCH: &str = "diff --git a/f.rs b/f.rs\n\
                         --- a/f.rs\n\
                         +++ b/f.rs\n\
                         @@ -1 +1 @@\n\
                         -one\n\
                         +two\n";

    fn command(words: &[&str]) -> Command {
        let mut command = Command::new(words[0]);
        command.args(&words[1..]);
        command
    }

    fn finished(input: Input, renderer: Option<Command>, limit: u64) -> Snapshot {
        let stream = Stream::start(input, renderer, limit).unwrap();
        stream.wait_for(usize::MAX);
        stream.snapshot()
    }

    #[test]
    fn everything_arrives_once_done() {
        let snapshot = finished(
            Input::Text("a\nb\n".into()),
            Some(command(&["sed", "s/^/drawn /"])),
            1_000,
        );

        assert!(snapshot.done);
        assert_eq!(snapshot.text, "a\nb\n");
        assert_eq!(snapshot.rendered.as_deref(), Some("drawn a\ndrawn b\n"));
        assert!(!snapshot.truncated);
    }

    /// The point of it: the first rows are there to show while the renderer is
    /// still drawing the rest.
    #[test]
    fn the_first_rows_arrive_before_the_renderer_finishes() {
        let renderer = command(&["awk", r#"{print} NR==2{fflush(); system("sleep 3")}"#]);
        let started = Instant::now();
        let stream =
            Stream::start(Input::Text("a\nb\nc\nd\n".into()), Some(renderer), 1_000).unwrap();

        stream.wait_for(2);
        let snapshot = stream.snapshot();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!snapshot.done);
        assert_eq!(snapshot.rendered.as_deref(), Some("a\nb\n"));
    }

    /// While a command is still writing, what it has written so far is cut back
    /// to its last whole hunk.
    #[test]
    fn text_still_being_written_is_cut_back_to_whole_hunks() {
        let writer = command(&[
            "sh",
            "-c",
            r#"printf '%s@@ -9 +9 @@\n-thr' "$0"; sleep 3; printf 'ee\n+four\n'"#,
            PATCH,
        ]);
        let stream = Stream::start(Input::Command(writer), Some(command(&["cat"])), 1_000).unwrap();

        stream.wait_for(PATCH.lines().count() + 1);
        let snapshot = stream.snapshot();

        assert!(!snapshot.done);
        assert_eq!(snapshot.text, PATCH);
    }

    #[test]
    fn reading_stops_at_the_limit() {
        let text = format!("{PATCH}@@ -9 +9 @@\n-three\n+four\n");
        let snapshot = finished(
            Input::Text(text.clone()),
            Some(command(&["cat"])),
            (text.len() - 1) as u64,
        );

        assert!(snapshot.done);
        assert!(snapshot.truncated);
        assert_eq!(snapshot.text, PATCH);
    }

    #[test]
    fn there_need_be_no_limit() {
        let snapshot = finished(Input::Text("a\n".into()), None, u64::MAX);

        assert!(snapshot.done);
        assert_eq!(snapshot.text, "a\n");
    }

    #[test]
    fn without_a_renderer_the_text_is_all_there_is() {
        let snapshot = finished(Input::Text("a\n".into()), None, 1_000);

        assert!(snapshot.done);
        assert_eq!(snapshot.text, "a\n");
        assert_eq!(snapshot.rendered, None);
    }

    /// A command whose output is already drawn, as a log command piping into
    /// delta is, needs no renderer: its own rows are shown as they arrive.
    #[test]
    fn without_a_renderer_the_first_rows_arrive_before_the_command_finishes() {
        let writer = command(&["sh", "-c", r#"printf 'a\nb\n'; sleep 3; printf 'c\n'"#]);
        let started = Instant::now();
        let stream = Stream::start(Input::Command(writer), None, 1_000).unwrap();

        stream.wait_for(2);
        let snapshot = stream.snapshot();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!snapshot.done);
        assert_eq!(snapshot.text, "a\nb\n");
    }

    #[test]
    fn a_command_that_fails_having_written_says_so() {
        let snapshot = finished(
            Input::Command(command(&["sh", "-c", "echo out; exit 3"])),
            None,
            1_000,
        );

        assert!(snapshot.failed);
        assert_eq!(snapshot.refused, None);
        assert_eq!(snapshot.text, "out\n");
    }

    #[test]
    fn a_renderer_that_fails_has_drawn_nothing() {
        let snapshot = finished(
            Input::Text("a\n".into()),
            Some(command(&["sh", "-c", "cat >/dev/null; exit 1"])),
            1_000,
        );

        assert!(snapshot.done);
        assert_eq!(snapshot.rendered, None);
    }

    #[test]
    fn a_command_git_refuses_says_why() {
        let snapshot = finished(
            Input::Command(command(&["sh", "-c", "echo 'fatal: no' >&2; exit 128"])),
            Some(command(&["cat"])),
            1_000,
        );

        assert!(snapshot.done);
        assert_eq!(snapshot.refused.as_deref(), Some("fatal: no"));
    }
}
