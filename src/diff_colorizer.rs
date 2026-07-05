//! Colorize diffs with an external command (e.g. `delta`).
//!
//! The command is fed a unified diff on stdin and emits ANSI-colored output that
//! we parse back into per-line styled runs.
//!
//! Two cooperation levels are supported:
//! - A `--color-only` renderer preserves line structure 1:1, and
//!   [`crate::highlight`] maps its runs onto hunk content lines by position.
//! - A renderer speaking the OSC-1717 diff-line-metadata protocol may restructure
//!   the diff freely (drop `+`/`-` markers, side-by-side, gutters) and annotate
//!   each rendered line with its patch-space identity `(file, kind, new/old line)`.
//!   We advertise the protocol via the `OSC1717_METADATA` env var and attach each
//!   record to the line that follows it.

use anstyle_parse::{DefaultCharAccumulator, Params, Parser, Perform};
use ratatui::style::{Color, Modifier, Style};
use std::io::Write;
use std::mem;
use std::ops::Range;
use std::process::{Command, Stdio};
use std::thread;

/// Protocol versions of the OSC-1717 diff-line-metadata spec we understand,
/// advertised to the renderer via `OSC1717_METADATA` (a version set, `V`-prefixed).
const OSC1717_METADATA_ADVERTISED: &str = "V1";

/// Which side of the diff a rendered content line represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    Context,
    Added,
    Deleted,
}

/// The patch-space identity of a rendered line, recovered from its OSC-1717
/// record. `old_line` is present only for deletions (see spec §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineMetadata {
    pub kind: LineKind,
    pub new_line: u32,
    pub old_line: Option<u32>,
    pub file: String,
}

/// A single line of colorizer output: its plain text (ANSI stripped, newline
/// excluded), the contiguous `(byte-range, style)` runs that tile it, and its
/// OSC-1717 identity if the renderer emitted one for it.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ParsedLine {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
    pub metadata: Option<LineMetadata>,
}

/// The result of parsing a renderer's output.
#[derive(Debug, Default)]
pub(crate) struct ParsedOutput {
    pub lines: Vec<ParsedLine>,
    /// The OSC-1717 protocol version the renderer negotiated, if it spoke the
    /// protocol at all (from the handshake record, spec §4.4). Consumed by the
    /// rendering integration to select metadata mode.
    #[allow(dead_code)]
    pub protocol_version: Option<u32>,
}

/// Run the colorizer `command`, feeding `input` on stdin and returning its
/// stdout. `None` if the command can't be spawned or exits non-zero.
pub(crate) fn run(command: &[String], input: &str) -> Option<String> {
    let (program, args) = command.split_first()?;

    let mut child = Command::new(program)
        .args(args)
        .env("OSC1717_METADATA", OSC1717_METADATA_ADVERTISED)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .inspect_err(|e| log::warn!("diff colorizer '{program}' failed to spawn: {e}"))
        .ok()?;

    // Write stdin on a separate thread so a full stdout pipe can't deadlock us.
    let mut stdin = child.stdin.take()?;
    let input = input.to_owned();
    let writer = thread::spawn(move || stdin.write_all(input.as_bytes()));

    let output = child.wait_with_output().ok()?;
    let _ = writer.join();

    if !output.status.success() {
        log::warn!("diff colorizer '{program}' exited with {}", output.status);
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse ANSI-colored text into per-line styled runs, capturing any OSC-1717
/// diff-line-metadata records the renderer emitted.
pub(crate) fn parse_ansi_lines(output: &str) -> ParsedOutput {
    let mut performer = Performer::default();
    let mut parser = Parser::<DefaultCharAccumulator>::new();
    for &byte in output.as_bytes() {
        parser.advance(&mut performer, byte);
    }
    performer.finish()
}

#[derive(Default)]
struct Performer {
    lines: Vec<ParsedLine>,
    style: Style,
    text: String,
    runs: Vec<(Range<usize>, Style)>,
    /// Start offset and style of the run currently being accumulated.
    open: Option<(usize, Style)>,
    /// The first OSC-1717 record seen on the current line (row-granular: the
    /// first record wins, which in side-by-side is the left/old column).
    metadata: Option<LineMetadata>,
    /// Set when the current line carries only the version-only handshake record,
    /// so we drop that empty line rather than render it.
    handshake_line: bool,
    protocol_version: Option<u32>,
}

impl Performer {
    fn end_line(&mut self) {
        if let Some((start, style)) = self.open.take() {
            self.runs.push((start..self.text.len(), style));
        }
        let handshake_only = mem::take(&mut self.handshake_line) && self.text.is_empty();
        let line = ParsedLine {
            text: mem::take(&mut self.text),
            runs: mem::take(&mut self.runs),
            metadata: self.metadata.take(),
        };
        if !handshake_only {
            self.lines.push(line);
        }
    }

    fn finish(mut self) -> ParsedOutput {
        if !self.text.is_empty() || self.open.is_some() || self.metadata.is_some() {
            self.end_line();
        }
        ParsedOutput {
            lines: self.lines,
            protocol_version: self.protocol_version,
        }
    }
}

impl Perform for Performer {
    fn print(&mut self, c: char) {
        match self.open {
            Some((_, style)) if style == self.style => {}
            Some((start, style)) => {
                self.runs.push((start..self.text.len(), style));
                self.open = Some((self.text.len(), self.style));
            }
            None => self.open = Some((self.text.len(), self.style)),
        }
        self.text.push(c);
    }

    fn execute(&mut self, byte: u8) {
        // Tabs arrive as a C0 control but are content in a diff; keep them. `\r`
        // and other controls are dropped (the visual line excludes them too).
        match byte {
            b'\n' => self.end_line(),
            b'\t' => self.print('\t'),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: u8) {
        if action == b'm' {
            let codes: Vec<u16> = params
                .iter()
                .flat_map(|group| group.iter().copied())
                .collect();
            apply_sgr(&mut self.style, &codes);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.first() != Some(&b"1717".as_slice()) {
            return;
        }
        match params {
            // Handshake (spec §4.4): version only, no further fields.
            [_, version] => {
                self.protocol_version = parse_u32(version);
                self.handshake_line = true;
            }
            // Per-line record: version, type, new-line, old-line, file… (the file
            // is last and may itself contain ';', so rejoin the tail — spec §4.1).
            [_, version, kind, new_line, old_line, file_head @ ..] if !file_head.is_empty() => {
                self.protocol_version = self.protocol_version.or_else(|| parse_u32(version));
                if self.metadata.is_none()
                    && let Some(kind) = line_kind(kind)
                {
                    self.metadata = Some(LineMetadata {
                        kind,
                        new_line: parse_u32(new_line).unwrap_or(0),
                        old_line: parse_u32(old_line),
                        file: file_head
                            .iter()
                            .map(|part| String::from_utf8_lossy(part))
                            .collect::<Vec<_>>()
                            .join(";"),
                    });
                }
            }
            _ => {}
        }
    }
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn line_kind(bytes: &[u8]) -> Option<LineKind> {
    match bytes {
        b"c" => Some(LineKind::Context),
        b"a" => Some(LineKind::Added),
        b"d" => Some(LineKind::Deleted),
        _ => None, // Unknown type: treat the row as non-actionable (spec §5.1).
    }
}

/// Apply an SGR (Select Graphic Rendition) parameter list to `style`.
fn apply_sgr(style: &mut Style, codes: &[u16]) {
    if codes.is_empty() {
        *style = Style::new();
        return;
    }

    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => *style = Style::new(),
            1 => style.add_modifier |= Modifier::BOLD,
            2 => style.add_modifier |= Modifier::DIM,
            3 => style.add_modifier |= Modifier::ITALIC,
            4 => style.add_modifier |= Modifier::UNDERLINED,
            7 => style.add_modifier |= Modifier::REVERSED,
            9 => style.add_modifier |= Modifier::CROSSED_OUT,
            22 => style.add_modifier.remove(Modifier::BOLD | Modifier::DIM),
            23 => style.add_modifier.remove(Modifier::ITALIC),
            24 => style.add_modifier.remove(Modifier::UNDERLINED),
            27 => style.add_modifier.remove(Modifier::REVERSED),
            29 => style.add_modifier.remove(Modifier::CROSSED_OUT),
            c @ 30..=37 => style.fg = Some(Color::Indexed((c - 30) as u8)),
            38 => match parse_extended_color(&codes[i + 1..]) {
                Some((color, advance)) => {
                    style.fg = Some(color);
                    i += advance;
                }
                None => break,
            },
            39 => style.fg = None,
            c @ 40..=47 => style.bg = Some(Color::Indexed((c - 40) as u8)),
            48 => match parse_extended_color(&codes[i + 1..]) {
                Some((color, advance)) => {
                    style.bg = Some(color);
                    i += advance;
                }
                None => break,
            },
            49 => style.bg = None,
            c @ 90..=97 => style.fg = Some(Color::Indexed((c - 90 + 8) as u8)),
            c @ 100..=107 => style.bg = Some(Color::Indexed((c - 100 + 8) as u8)),
            _ => {}
        }
        i += 1;
    }
}

/// Parse the tail of a `38`/`48` extended-color sequence, returning the color and
/// how many parameters after the `38`/`48` it consumed.
fn parse_extended_color(rest: &[u16]) -> Option<(Color, usize)> {
    match rest.first()? {
        2 => Some((
            Color::Rgb(
                *rest.get(1)? as u8,
                *rest.get(2)? as u8,
                *rest.get(3)? as u8,
            ),
            4,
        )),
        5 => Some((Color::Indexed(*rest.get(1)? as u8), 2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn parses_lines_tiling_text_with_colors() {
        // Red "fn", default " main"; then a plain line.
        let output = "\x1b[38;2;255;0;0mfn\x1b[0m main\nplain line\n";
        let lines = parse_ansi_lines(output).lines;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "fn main");
        assert_eq!(lines[1].text, "plain line");

        // Runs must tile the whole line contiguously from 0.
        for line in &lines {
            let mut next = 0;
            for (range, _) in &line.runs {
                assert_eq!(range.start, next, "runs must be contiguous with no gaps");
                next = range.end;
            }
            assert_eq!(next, line.text.len(), "runs must cover the whole line");
        }

        let fg = lines[0]
            .runs
            .iter()
            .find(|(r, _)| lines[0].text[r.clone()] == *"fn")
            .map(|(_, s)| s.fg);
        assert_eq!(fg, Some(Some(Color::Rgb(255, 0, 0))));
    }

    #[test]
    fn preserves_tabs_and_markers() {
        let lines = parse_ansi_lines("\x1b[32m+\tlet x = 1;\x1b[0m\n").lines;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "+\tlet x = 1;");
    }

    #[test]
    fn captures_osc1717_handshake_and_drops_its_line() {
        // Handshake record (version only) precedes the content, on its own line.
        let out = parse_ansi_lines("\x1b]1717;1\x1b\\\ncontent\n");
        assert_eq!(out.protocol_version, Some(1));
        // The empty handshake line is dropped; only real content remains.
        assert_eq!(out.lines.len(), 1);
        assert_eq!(out.lines[0].text, "content");
        assert_eq!(out.lines[0].metadata, None);
    }

    #[test]
    fn attaches_per_line_records_to_following_line() {
        let out = parse_ansi_lines(
            "\x1b]1717;1\x1b\\\n\
             \x1b]1717;1;c;8;;src/foo.rs\x1b\\fn a() {}\n\
             \x1b]1717;1;d;10;11;src/foo.rs\x1b\\    let old = 1;\n\
             \x1b]1717;1;a;10;;src/foo.rs\x1b\\    let new = 1;\n",
        )
        .lines;

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "fn a() {}");
        assert_eq!(
            out[0].metadata,
            Some(LineMetadata {
                kind: LineKind::Context,
                new_line: 8,
                old_line: None,
                file: "src/foo.rs".into(),
            })
        );
        assert_eq!(
            out[1].metadata,
            Some(LineMetadata {
                kind: LineKind::Deleted,
                new_line: 10,
                old_line: Some(11),
                file: "src/foo.rs".into(),
            })
        );
        assert_eq!(out[2].metadata.as_ref().unwrap().kind, LineKind::Added);
    }

    #[test]
    fn first_record_wins_for_side_by_side_row() {
        // A side-by-side row carries two records (left=old/deleted, right=new/added);
        // the row-granular identity is the first (left) one.
        let out = parse_ansi_lines(
            "\x1b]1717;1;d;10;10;f.rs\x1b\\ old \x1b]1717;1;a;10;;f.rs\x1b\\ new \n",
        )
        .lines;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, " old  new ");
        assert_eq!(out[0].metadata.as_ref().unwrap().kind, LineKind::Deleted);
        assert_eq!(out[0].metadata.as_ref().unwrap().old_line, Some(10));
    }

    #[test]
    fn file_field_may_contain_semicolons() {
        let out = parse_ansi_lines("\x1b]1717;1;a;1;;weird;name.txt\x1b\\x\n").lines;
        assert_eq!(out[0].metadata.as_ref().unwrap().file, "weird;name.txt");
    }
}
