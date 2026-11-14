//! A command's output piped through the renderer as it is written, as a shell
//! pipeline would: what the renderer has drawn can be shown while the rest of
//! it is still being written and drawn.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Where the text comes from: a command gitu runs, or text already in hand.
pub(crate) enum Input {
    Command(Command),
    Text(String),
    #[cfg(test)]
    Reader(Box<dyn Read + Send>),
}

/// A pipeline from an [`Input`] through a renderer, read on threads of its own.
/// Dropping it stops reading the input, killing a command still running; what
/// has been read is still drawn.
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
    /// How many bytes of the input the renderer has taken; all of them without
    /// a renderer.
    taken: usize,
    /// What the renderer drew, up to its last whole row. `None` without a
    /// renderer, or when it could not draw.
    pub rendered: Option<String>,
    /// Whether the input and the renderer have both finished.
    pub done: bool,
    /// What git said when it exited unsuccessfully having written nothing.
    pub refused: Option<String>,
    /// What to tell the user when the renderer exited unsuccessfully.
    pub renderer_failed: Option<String>,
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
    has_renderer: bool,
    rendered: Option<Vec<u8>>,
    rendered_rows: usize,
    rendered_done: bool,
    renderer_failed: Option<String>,
    /// How many bytes of the input have been written to the renderer.
    fed: usize,
    abandoned: bool,
    /// The input's command, until it closes its output and is waited for.
    command: Option<Child>,
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
        change(&mut self.lock());
        self.changed.notify_all();
    }

    /// The state, even after a thread panicked holding it: the threads only
    /// ever add to it, so what is there is still worth showing.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Applies its change to the state when dropped, so that a thread that ends by
/// panicking still says it has finished.
struct OnExit<'a>(&'a Shared, fn(&mut State));

impl Snapshot {
    /// The text the renderer has taken, cut back to what is whole. Building
    /// from it rather than from all of the text is cheaper when what the
    /// renderer has not reached is not shown.
    pub(crate) fn taken(&self) -> &str {
        let end = (0..=self.taken.min(self.text.len()))
            .rev()
            .find(|&end| self.text.is_char_boundary(end))
            .unwrap_or_default();
        crate::capped_input::whole(&self.text[..end])
    }
}

impl Drop for OnExit<'_> {
    fn drop(&mut self) {
        self.0.update(self.1);
    }
}

impl Stream {
    /// Start `input` writing into `renderer`, reading at most `limit` bytes of
    /// it. Without a renderer, or with one that cannot be started (which is
    /// said, as a renderer failing is), the input is only read.
    pub(crate) fn start(input: Input, renderer: Option<Command>, limit: u64) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });

        let (source, stderr): (Box<dyn Read + Send>, Option<ChildStderr>) = match input {
            Input::Text(text) => (Box::new(io::Cursor::new(text.into_bytes())), None),
            #[cfg(test)]
            Input::Reader(reader) => (reader, None),
            Input::Command(mut command) => {
                let mut child = command
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                let stdout = child.stdout.take().expect("stdout was piped");
                let stderr = child.stderr.take();
                shared.update(|state| state.command = Some(child));
                (Box::new(stdout), stderr)
            }
        };

        let renderer_name = renderer
            .as_ref()
            .map(|command| command.get_program().to_owned());
        let mut renderer = renderer.and_then(|mut command| {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .inspect_err(|e| {
                    log::warn!("renderer failed to spawn: {e}");
                    let failed =
                        crate::diff_renderer::not_started_message(command.get_program(), e);
                    shared.update(|state| state.renderer_failed = Some(failed));
                })
                .ok()
        });
        let sink = renderer.as_mut().and_then(|child| child.stdin.take());

        match renderer {
            Some(mut child) => {
                let stdout = child.stdout.take().expect("stdout was piped");
                let name = renderer_name.unwrap_or_default();
                shared.update(|state| {
                    state.has_renderer = true;
                    state.rendered = Some(Vec::new());
                });
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let _done = OnExit(&shared, |state| state.rendered_done = true);
                    draw(&shared, stdout, child, &name);
                });
            }
            None => shared.update(|state| state.rendered_done = true),
        }

        let reader = Arc::clone(&shared);
        thread::spawn(move || {
            let _done = OnExit(&reader, |state| {
                state.input_done = true;
                if let Some(command) = &mut state.command {
                    let _ = command.kill();
                }
            });
            read(&reader, source, sink, stderr, limit);
        });

        Ok(Self { shared })
    }

    /// Wait until there are `rows` rows to show, or everything is done.
    pub(crate) fn wait_for(&self, rows: usize) {
        let mut state = self.shared.lock();
        while !state.done() && state.rows() < rows {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// [`Self::wait_for`], giving up after `timeout`.
    pub(crate) fn wait_at_most(&self, rows: usize, timeout: Duration) {
        let state = self.shared.lock();
        let _ = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| !state.done() && state.rows() < rows)
            .unwrap_or_else(PoisonError::into_inner);
    }

    /// How many rows there are to show so far.
    pub(crate) fn rows(&self) -> usize {
        self.shared.lock().rows()
    }

    pub(crate) fn is_done(&self) -> bool {
        self.shared.lock().done()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.shared.lock();
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
            taken: if state.has_renderer {
                state.fed
            } else {
                state.input.len()
            },
            text,
            truncated: state.truncated,
            rendered,
            done,
            refused: state.refused.clone(),
            renderer_failed: state.renderer_failed.clone(),
        }
    }
}

/// Streams shown one after another as if they were one: the rows to show are
/// those of each stream that is done, and of the first that is not.
pub(crate) struct Streams(pub(crate) Vec<Stream>);

impl Streams {
    /// Wait until there are `rows` rows to show, or everything is done.
    pub(crate) fn wait_for(&self, rows: usize) {
        let mut before = 0;
        for stream in &self.0 {
            stream.wait_for(rows.saturating_sub(before));
            if !stream.is_done() {
                return;
            }
            before += stream.rows();
        }
    }

    /// [`Self::wait_for`], giving up after `timeout`.
    pub(crate) fn wait_at_most(&self, rows: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut before = 0;
        for stream in &self.0 {
            stream.wait_at_most(
                rows.saturating_sub(before),
                deadline.saturating_duration_since(Instant::now()),
            );
            if !stream.is_done() {
                return;
            }
            before += stream.rows();
        }
    }

    /// How many rows there are to show so far.
    pub(crate) fn rows(&self) -> usize {
        let mut rows = 0;
        for stream in &self.0 {
            rows += stream.rows();
            if !stream.is_done() {
                break;
            }
        }
        rows
    }

    pub(crate) fn is_done(&self) -> bool {
        self.0.iter().all(Stream::is_done)
    }

    pub(crate) fn snapshots(&self) -> Vec<Snapshot> {
        self.0.iter().map(Stream::snapshot).collect()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.update(|state| {
            state.abandoned = true;
            if let Some(command) = &mut state.command {
                let _ = command.kill();
            }
        });
    }
}

/// Read `source` into the shared state and on into the renderer, up to one
/// byte past `limit` (so that stopping there is distinguishable from input
/// exactly that long). A command still writing past the limit is killed. The
/// renderer is fed on a thread of its own, so the input is read as fast as it
/// is written rather than as fast as the renderer takes it.
fn read(
    shared: &Arc<Shared>,
    mut source: Box<dyn Read + Send>,
    sink: Option<ChildStdin>,
    stderr: Option<ChildStderr>,
    limit: u64,
) {
    // Read stderr on its own thread: a command filling that pipe while gitu is
    // reading stdout would otherwise deadlock.
    let complaint = stderr.map(|mut pipe| {
        thread::spawn(move || {
            let mut said = String::new();
            let _ = pipe.read_to_string(&mut said);
            said
        })
    });

    let feeder = sink.map(|stdin| {
        let (sender, chunks) = mpsc::channel();
        let shared = Arc::clone(shared);
        thread::spawn(move || feed(&shared, stdin, chunks));
        sender
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

        if let Some(feeder) = &feeder {
            let _ = feeder.send(chunk.to_vec());
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
    drop(feeder);

    let mut refused = None;
    let mut command = None;
    shared.update(|state| command = state.command.take());
    if let Some(mut child) = command {
        if stopped {
            // The rest of it is not going to be read, so it is not left to be
            // written either.
            let _ = child.kill();
        }
        let succeeded = child.wait().is_ok_and(|status| status.success());
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
        state.input_done = true;
    });
}

/// Write `chunks` to the renderer as they arrive, until it stops taking them.
fn feed(shared: &Shared, mut stdin: ChildStdin, chunks: Receiver<Vec<u8>>) {
    for chunk in chunks {
        if stdin.write_all(&chunk).is_err() {
            return;
        }
        shared.lock().fed += chunk.len();
    }
}

/// Read the renderer's output into the shared state as it draws it. `program`
/// is the renderer's, for saying what went wrong if it fails.
fn draw(shared: &Shared, mut stdout: ChildStdout, mut child: Child, program: &OsStr) {
    // Read stderr on its own thread, as for the input's command.
    let complaint = child.stderr.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut said = String::new();
            let _ = pipe.read_to_string(&mut said);
            said
        })
    });
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

    let status = child.wait();
    let said = complaint
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    let failed = match status {
        Ok(status) if status.success() => None,
        Ok(status) => Some(crate::diff_renderer::failure_message(
            program, &said, status,
        )),
        Err(e) => Some(crate::diff_renderer::failure_message(program, "", e)),
    };
    if let Some(failed) = &failed {
        log::warn!("renderer exited unsuccessfully: {failed}");
    }
    shared.update(|state| {
        if let Some(failed) = failed {
            state.rendered = None;
            state.renderer_failed = Some(failed);
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

    /// The input is read as fast as it is written, not as fast as the renderer
    /// takes it: what the files are is known before the renderer reaches them.
    #[test]
    fn the_input_is_read_before_the_renderer_takes_it() {
        let text = PATCH.repeat(20_000);
        let renderer = command(&["sh", "-c", "sleep 3; cat"]);
        let started = Instant::now();
        let stream = Stream::start(Input::Text(text.clone()), Some(renderer), u64::MAX).unwrap();

        while stream.snapshot().text.len() < text.len() {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "the input waited for the renderer"
            );
            thread::sleep(Duration::from_millis(10));
        }
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
    fn a_renderer_that_fails_has_drawn_nothing() {
        let snapshot = finished(
            Input::Text("a\n".into()),
            Some(command(&["sh", "-c", "cat >/dev/null; exit 1"])),
            1_000,
        );

        assert!(snapshot.done);
        assert_eq!(snapshot.rendered, None);
    }

    /// A renderer that cannot be started has drawn nothing, and says why.
    #[test]
    fn a_renderer_that_cannot_be_started_says_why() {
        let snapshot = finished(
            Input::Text("a\n".into()),
            Some(command(&["/no/such/renderer"])),
            1_000,
        );

        assert_eq!(snapshot.rendered, None);
        let failed = snapshot.renderer_failed.unwrap_or_default();
        assert!(
            failed.starts_with("renderer could not be started: "),
            "{failed}"
        );
        assert_eq!(snapshot.text, "a\n");
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

    #[test]
    fn a_command_that_fails_having_written_is_not_refused() {
        let snapshot = finished(
            Input::Command(command(&["sh", "-c", "echo out; exit 3"])),
            None,
            1_000,
        );

        assert_eq!(snapshot.refused, None);
        assert_eq!(snapshot.text, "out\n");
    }

    /// A refresh drops the stream it replaces. A command still writing must
    /// not be left blocked on a pipe nobody reads, nor a silent one left to
    /// run on.
    #[test]
    fn dropping_a_stream_stops_a_command_still_writing() {
        assert_stopped_when_dropped("echo $$; exec yes");
    }

    #[test]
    fn dropping_a_stream_stops_a_command_yet_to_write_more() {
        assert_stopped_when_dropped("echo $$; exec sleep 60");
    }

    /// Start `script`, which first writes its pid, and drop its stream.
    fn assert_stopped_when_dropped(script: &str) {
        let stream = Stream::start(
            Input::Command(command(&["sh", "-c", script])),
            None,
            u64::MAX,
        )
        .unwrap();
        stream.wait_for(1);
        let pid = stream
            .snapshot()
            .text
            .trim()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        drop(stream);

        let started = Instant::now();
        while running(&pid) && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(50));
        }
        let stopped = !running(&pid);
        let _ = Command::new("kill").arg(&pid).status();
        assert!(stopped, "still running");
    }

    /// Whether `pid` is a process that has not been reaped.
    fn running(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// A thread that dies part way must not leave the view waiting forever:
    /// what was read before it died is shown.
    #[test]
    fn a_stream_whose_reading_panics_is_done() {
        struct Panics;
        impl Read for Panics {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("reading panicked");
            }
        }
        let stream = Stream::start(Input::Reader(Box::new(Panics)), None, u64::MAX).unwrap();

        let (done, waited) = std::sync::mpsc::channel();
        thread::spawn(move || {
            stream.wait_for(usize::MAX);
            let _ = done.send(stream.snapshot().done);
        });
        assert_eq!(waited.recv_timeout(Duration::from_secs(5)), Ok(true));
    }
}
