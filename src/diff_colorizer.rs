//! Colorize diffs with an external command (e.g. `delta`).
//!
//! The command is fed a unified diff on stdin and must emit ANSI-colored output
//! that preserves line structure. We parse that output back into per-line styled
//! runs, which [`crate::highlight`] maps onto hunk content lines.

use anstyle_parse::{DefaultCharAccumulator, Params, Parser, Perform};
use ratatui::style::{Color, Modifier, Style};
use std::io::Write;
use std::mem;
use std::ops::Range;
use std::process::{Command, Stdio};
use std::thread;

/// A single line of colorizer output: its plain text (ANSI stripped, newline
/// excluded) and the contiguous `(byte-range, style)` runs that tile it.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ParsedLine {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
}

/// Run the colorizer `command`, feeding `input` on stdin and returning its
/// stdout. `None` if the command can't be spawned or exits non-zero.
pub(crate) fn run(command: &[String], input: &str) -> Option<String> {
    let (program, args) = command.split_first()?;

    let mut child = Command::new(program)
        .args(args)
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

/// Parse ANSI-colored text into per-line styled runs.
pub(crate) fn parse_ansi_lines(output: &str) -> Vec<ParsedLine> {
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
}

impl Performer {
    fn end_line(&mut self) {
        if let Some((start, style)) = self.open.take() {
            self.runs.push((start..self.text.len(), style));
        }
        self.lines.push(ParsedLine {
            text: mem::take(&mut self.text),
            runs: mem::take(&mut self.runs),
        });
    }

    fn finish(mut self) -> Vec<ParsedLine> {
        if !self.text.is_empty() || self.open.is_some() {
            self.end_line();
        }
        self.lines
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
        let lines = parse_ansi_lines(output);

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
        let lines = parse_ansi_lines("\x1b[32m+\tlet x = 1;\x1b[0m\n");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "+\tlet x = 1;");
    }
}
