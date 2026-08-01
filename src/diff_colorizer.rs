//! Colorize diffs with an external command (e.g. `delta`).
//!
//! The command is fed a unified diff on stdin and emits ANSI-colored output that
//! we parse back into per-line styled runs. The same ANSI/OSC-1717 parsing drives
//! the log view, whose rows come from a `git log` command instead of stdin (see
//! [`COMMIT_RECORD_FORMAT`]).
//!
//! Two cooperation levels are supported:
//! - A `--color-only` renderer preserves line structure 1:1, and
//!   [`crate::highlight`] maps its runs onto hunk content lines by position.
//! - A renderer speaking the OSC-1717 diff-line-metadata protocol may restructure
//!   the diff freely (drop `+`/`-` markers, side-by-side, gutters) and annotate
//!   each rendered line with its patch-space identity `(file, kind, new/old line)`.
//!   We advertise the protocol via the `OSC1717_METADATA` env var and attach each
//!   record to the line that follows it.

use crate::git::diff::Diff;
use anstyle_parse::{DefaultCharAccumulator, Params, Parser, Perform};
use ratatui::style::{Color, Modifier, Style};
use std::io::Write;
use std::mem;
use std::ops::Range;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Protocol versions of the OSC-1717 diff-line-metadata spec we understand,
/// advertised to the renderer via `OSC1717_METADATA` (a version set, `V`-prefixed).
const OSC1717_METADATA_ADVERTISED: &str = "V1";

/// The git `--format` directives that emit a commit record: gitu substitutes
/// them for the `{commit}` token in the configured log command, so git itself
/// states which commit each rendered row belongs to.
pub(crate) const COMMIT_RECORD_FORMAT: &str = "%x1b]1717;1;C;;;%H%x1b\\";

/// What a rendered row is, per its OSC-1717 `type`. `Context`/`Added`/`Deleted`
/// are content lines; `HunkHeader`/`FileHeader` are the renderer's own header rows
/// (spec §12), which the host may display in place of drawing its own. `Commit`
/// (`C`) marks the first row of a commit in rendered log output; it carries the
/// commit id in the record's last field, where a diff row carries its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    Context,
    Added,
    Deleted,
    HunkHeader,
    FileHeader,
    Commit,
}

/// A run of rendered log rows belonging to one commit, as split by the `C`
/// records: a record starts a new block and the rows that follow it, up to the
/// next record, are that commit's.
#[derive(Debug, PartialEq)]
pub(crate) struct CommitBlock<'a> {
    /// The commit id, or `None` for rows preceding the first record.
    pub commit: Option<&'a str>,
    pub rows: &'a [ParsedLine],
}

/// Split rendered log rows into per-commit blocks.
pub(crate) fn commit_blocks(lines: &[ParsedLine]) -> Vec<CommitBlock<'_>> {
    let mut starts: Vec<(usize, Option<&str>)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(record) = line
            .records
            .iter()
            .find(|record| record.kind == LineKind::Commit)
        {
            starts.push((i, Some(record.file.as_str())));
        } else if starts.is_empty() {
            starts.push((0, None));
        }
    }

    starts
        .iter()
        .enumerate()
        .map(|(n, &(start, commit))| CommitBlock {
            commit,
            rows: &lines[start..starts.get(n + 1).map_or(lines.len(), |&(next, _)| next)],
        })
        .collect()
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
/// excluded), the contiguous `(byte-range, style)` runs that tile it, and the
/// OSC-1717 records emitted for it. A row usually has one record; a side-by-side
/// row that fuses a change carries two (left = deletion, right = addition), and
/// decoration rows carry none. The first record is the row-granular identity;
/// all of them are the lines a row-granular action operates on.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ParsedLine {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
    pub records: Vec<LineMetadata>,
}

/// The result of parsing a renderer's output.
#[derive(Debug, Default)]
pub(crate) struct ParsedOutput {
    pub lines: Vec<ParsedLine>,
    /// The OSC-1717 protocol version the renderer negotiated, if it spoke the
    /// protocol at all (from the handshake record, spec §4.4). Consumed by the
    /// rendering integration to select metadata mode.
    pub protocol_version: Option<u32>,
}

/// Resolve a rendered line's OSC-1717 identity to its position in the parsed
/// diff: `(file_index, hunk_index, line_index)`, where `line_index` indexes the
/// hunk's content lines (as [`crate::items`] does). This is the bridge that lets
/// staging operate on a freely-restructured render — the parsed diff stays the
/// source of truth; the metadata only says which content line a row is.
///
/// Additions and context match on the new-file line number; deletions match on
/// the old-file line number (two consecutive deletions share a new-file number,
/// so only the old-file number distinguishes them — spec §5.3).
pub(crate) fn resolve_line(diff: &Diff, meta: &LineMetadata) -> Option<(usize, usize, usize)> {
    for (file_index, file_diff) in diff.file_diffs.iter().enumerate() {
        let new_path = file_diff.header.new_file.fmt(&diff.text);
        let old_path = file_diff.header.old_file.fmt(&diff.text);
        if meta.file != new_path && meta.file != old_path {
            continue;
        }

        for (hunk_index, hunk) in file_diff.hunks.iter().enumerate() {
            let content = &diff.text[hunk.content.range.clone()];
            let mut new_num = hunk.header.new_line_start;
            let mut old_num = hunk.header.old_line_start;

            for (line_index, line) in content.split_inclusive('\n').enumerate() {
                let found = match (meta.kind, line.chars().next()) {
                    (LineKind::Added, Some('+')) => new_num == meta.new_line,
                    (LineKind::Deleted, Some('-')) => Some(old_num) == meta.old_line,
                    (LineKind::Context, Some(' ')) => new_num == meta.new_line,
                    _ => false,
                };
                if found {
                    return Some((file_index, hunk_index, line_index));
                }

                match line.chars().next() {
                    Some('+') => new_num += 1,
                    Some('-') => old_num += 1,
                    Some('\\') => {} // "\ No newline at end of file": not a real line.
                    _ => {
                        new_num += 1;
                        old_num += 1;
                    }
                }
            }
        }
        return None; // File matched but the line wasn't found; don't scan others.
    }
    None
}

/// Resolve an `h` (hunk-header) record to `(file_index, hunk_index)` by matching
/// its file and its `new_line` (the hunk's first new-file line) against the parsed
/// diff's hunk headers. Lets the host render the renderer's hunk header as the
/// anchor for that hunk (collapse, whole-hunk staging) instead of drawing `@@`.
pub(crate) fn resolve_hunk(diff: &Diff, meta: &LineMetadata) -> Option<(usize, usize)> {
    for (file_index, file_diff) in diff.file_diffs.iter().enumerate() {
        if meta.file != file_diff.header.new_file.fmt(&diff.text)
            && meta.file != file_diff.header.old_file.fmt(&diff.text)
        {
            continue;
        }
        let hunk_index = file_diff
            .hunks
            .iter()
            .position(|hunk| hunk.header.new_line_start == meta.new_line)?;
        return Some((file_index, hunk_index));
    }
    None
}

/// Run `command` in `dir`, feeding it `input` on stdin (a diff to colorize; the
/// log command takes none) and returning its stdout. `None` if the command can't
/// be spawned or exits non-zero.
///
/// `width` is the number of columns gitu will render the output into. A renderer
/// that reflows (delta side-by-side/wrapping) can't detect this over a pipe and
/// defaults too narrow, so we make it available two ways: a literal `{width}`
/// token anywhere in the command is substituted (e.g. `delta --width {width}`),
/// and `COLUMNS` is exported for renderers that read it.
pub(crate) fn run(
    command: &[String],
    input: Option<&str>,
    width: usize,
    dir: Option<&Path>,
) -> Option<String> {
    let (program, args) = command.split_first()?;
    let args: Vec<String> = args
        .iter()
        .map(|arg| arg.replace("{width}", &width.to_string()))
        .collect();

    let mut command = Command::new(program);
    command
        .args(&args)
        .env("OSC1717_METADATA", OSC1717_METADATA_ADVERTISED)
        .env("COLUMNS", width.to_string())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    if let Some(dir) = dir {
        command.current_dir(dir);
    }

    let mut child = command
        .spawn()
        .inspect_err(|e| log::warn!("renderer '{program}' failed to spawn: {e}"))
        .ok()?;

    // Write stdin on a separate thread so a full stdout pipe can't deadlock us.
    let writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let input = input.to_owned();
        thread::spawn(move || stdin.write_all(input.as_bytes()))
    });

    let output = child.wait_with_output().ok()?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }

    if !output.status.success() {
        log::warn!("renderer '{program}' exited with {}", output.status);
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
    /// The OSC-1717 records seen on the current line, in emission order.
    records: Vec<LineMetadata>,
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
            records: mem::take(&mut self.records),
        };
        if !handshake_only {
            self.lines.push(line);
        }
    }

    fn finish(mut self) -> ParsedOutput {
        if !self.text.is_empty() || self.open.is_some() || !self.records.is_empty() {
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
                if let Some(kind) = line_kind(kind) {
                    self.records.push(LineMetadata {
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
        b"h" => Some(LineKind::HunkHeader),
        b"f" => Some(LineKind::FileHeader),
        b"C" => Some(LineKind::Commit),
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
        assert_eq!(out.lines[0].records, vec![]);
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
            out[0].records,
            vec![LineMetadata {
                kind: LineKind::Context,
                new_line: 8,
                old_line: None,
                file: "src/foo.rs".into(),
            }]
        );
        assert_eq!(
            out[1].records,
            vec![LineMetadata {
                kind: LineKind::Deleted,
                new_line: 10,
                old_line: Some(11),
                file: "src/foo.rs".into(),
            }]
        );
        assert_eq!(out[2].records[0].kind, LineKind::Added);
    }

    #[test]
    fn side_by_side_row_keeps_both_records_in_order() {
        // A side-by-side row that fuses a change carries two records (left =
        // deletion, right = addition); both are kept, first is the identity.
        let out = parse_ansi_lines(
            "\x1b]1717;1;d;10;10;f.rs\x1b\\ old \x1b]1717;1;a;10;;f.rs\x1b\\ new \n",
        )
        .lines;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, " old  new ");
        assert_eq!(out[0].records.len(), 2);
        assert_eq!(out[0].records[0].kind, LineKind::Deleted);
        assert_eq!(out[0].records[0].old_line, Some(10));
        assert_eq!(out[0].records[1].kind, LineKind::Added);
    }

    #[test]
    fn file_field_may_contain_semicolons() {
        let out = parse_ansi_lines("\x1b]1717;1;a;1;;weird;name.txt\x1b\\x\n").lines;
        assert_eq!(out[0].records[0].file, "weird;name.txt");
    }

    /// What `COMMIT_RECORD_FORMAT` expands to once git has run it.
    fn commit_record(oid: &str) -> String {
        format!("\x1b]1717;1;C;;;{oid}\x1b\\")
    }

    #[test]
    fn parses_commit_records() {
        let out = parse_ansi_lines(&format!(
            "{}▸ abc1234 summary\n",
            commit_record("abc1234def")
        ));
        assert_eq!(
            out.lines[0].records,
            vec![LineMetadata {
                kind: LineKind::Commit,
                new_line: 0,
                old_line: None,
                file: "abc1234def".into(),
            }]
        );
    }

    #[test]
    fn commit_blocks_start_at_each_commit_record() {
        // A commit's rows run from its record up to the next one; here each
        // commit's format emits the record on its own (blank) leading row.
        let out = parse_ansi_lines(&format!(
            "preamble\n\
             {}\n▸ aaa summary\n    body\n\
             {}\n▸ bbb summary\n",
            commit_record("aaa"),
            commit_record("bbb"),
        ));
        let blocks = commit_blocks(&out.lines);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].commit, None);
        assert_eq!(rows(&blocks[0]), vec!["preamble"]);
        assert_eq!(blocks[1].commit, Some("aaa"));
        assert_eq!(rows(&blocks[1]), vec!["", "▸ aaa summary", "    body"]);
        assert_eq!(blocks[2].commit, Some("bbb"));
        assert_eq!(rows(&blocks[2]), vec!["", "▸ bbb summary"]);
    }

    #[test]
    fn commit_blocks_of_unmarked_output_is_a_single_headless_block() {
        let out = parse_ansi_lines("no markers here\nat all\n");
        let blocks = commit_blocks(&out.lines);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].commit, None);
        assert_eq!(rows(&blocks[0]), vec!["no markers here", "at all"]);
    }

    fn rows<'a>(block: &'a CommitBlock) -> Vec<&'a str> {
        block.rows.iter().map(|row| row.text.as_str()).collect()
    }

    fn diff_from(text: &str) -> crate::git::diff::Diff {
        crate::git::diff::Diff {
            text: text.to_string(),
            diff_type: crate::git::diff::DiffType::WorkdirToIndex,
            file_diffs: crate::gitu_diff::Parser::new(text).parse_diff().unwrap(),
            commit: None,
        }
    }

    #[test]
    fn resolve_line_maps_identity_to_content_line() {
        let text = "diff --git a/src/foo.rs b/src/foo.rs\n\
                    index 1a2b3c4..5d6e7f8 100644\n\
                    --- a/src/foo.rs\n\
                    +++ b/src/foo.rs\n\
                    @@ -8,4 +8,4 @@ use std::io;\n\
                    \x20fn a() {}\n\
                    -    let old = 1;\n\
                    -    let old2 = 2;\n\
                    +    let new = 1;\n\
                    \x20ctx();\n";
        let diff = diff_from(text);
        let m = |kind, new_line, old_line| LineMetadata {
            kind,
            new_line,
            old_line,
            file: "src/foo.rs".into(),
        };

        // Content lines, in order: 0=context fn a, 1=del old, 2=del old2, 3=add new, 4=context ctx.
        assert_eq!(
            resolve_line(&diff, &m(LineKind::Context, 8, None)),
            Some((0, 0, 0))
        );
        // Consecutive deletions share new-line 9; only old-line tells them apart.
        assert_eq!(
            resolve_line(&diff, &m(LineKind::Deleted, 9, Some(9))),
            Some((0, 0, 1))
        );
        assert_eq!(
            resolve_line(&diff, &m(LineKind::Deleted, 9, Some(10))),
            Some((0, 0, 2))
        );
        assert_eq!(
            resolve_line(&diff, &m(LineKind::Added, 9, None)),
            Some((0, 0, 3))
        );
        assert_eq!(
            resolve_line(&diff, &m(LineKind::Context, 10, None)),
            Some((0, 0, 4))
        );
        // Unknown file / missing line resolve to None.
        assert_eq!(resolve_line(&diff, &m(LineKind::Added, 999, None)), None);
    }

    #[test]
    fn parses_hunk_and_file_header_records() {
        let out = parse_ansi_lines(
            "\x1b]1717;1;f;;;src/foo.rs\x1b\\file header\n\
             \x1b]1717;1;h;8;;src/foo.rs\x1b\\hunk header\n",
        )
        .lines;
        assert_eq!(out[0].records[0].kind, LineKind::FileHeader);
        assert_eq!(out[0].records[0].old_line, None);
        assert_eq!(out[1].records[0].kind, LineKind::HunkHeader);
        assert_eq!(out[1].records[0].new_line, 8);
    }

    #[test]
    fn resolve_hunk_matches_on_new_line_start() {
        let text = "diff --git a/f.rs b/f.rs\n\
                    index 1..2 100644\n\
                    --- a/f.rs\n\
                    +++ b/f.rs\n\
                    @@ -8,2 +8,2 @@\n\
                    -a\n\
                    +b\n\
                    @@ -40,2 +40,2 @@\n\
                    -c\n\
                    +d\n";
        let diff = diff_from(text);
        let h = |new_line| LineMetadata {
            kind: LineKind::HunkHeader,
            new_line,
            old_line: None,
            file: "f.rs".into(),
        };
        assert_eq!(resolve_hunk(&diff, &h(8)), Some((0, 0)));
        assert_eq!(resolve_hunk(&diff, &h(40)), Some((0, 1)));
        assert_eq!(resolve_hunk(&diff, &h(999)), None);
    }

    #[test]
    fn format_lines_patch_stages_a_fused_change_both_sides() {
        // A side-by-side row fusing a modified line maps to the deletion (index 1)
        // and its non-adjacent replacement (index 3). Staging that set must keep
        // both, leaving the other deletion (index 2) as context.
        let text = "diff --git a/src/foo.rs b/src/foo.rs\n\
                    index 1a2b3c4..5d6e7f8 100644\n\
                    --- a/src/foo.rs\n\
                    +++ b/src/foo.rs\n\
                    @@ -8,4 +8,3 @@ use std::io;\n\
                    \x20fn a() {}\n\
                    -    let old = 1;\n\
                    -    let old2 = 2;\n\
                    +    let new = 1;\n\
                    \x20ctx();\n";
        let diff = diff_from(text);

        let patch = diff.format_lines_patch(0, 0, &[1, 3], crate::git::diff::PatchMode::Normal);

        assert!(patch.contains("-    let old = 1;"), "keeps the deletion");
        assert!(patch.contains("+    let new = 1;"), "keeps the replacement");
        // The unselected deletion becomes context (leading space, no '-').
        assert!(
            patch.contains("     let old2 = 2;"),
            "other deletion -> context"
        );
        assert!(!patch.contains("-    let old2 = 2;"));
    }
}
