//! Colorize diffs with an external command (e.g. `delta`).
//!
//! The command is fed a unified diff on stdin and must emit ANSI-colored output
//! that preserves line structure. We parse that output back into per-line styled
//! runs, which [`crate::highlight`] maps onto hunk content lines.

use ratatui::style::Style;
use std::ops::Range;

/// A single line of colorizer output: its plain text (ANSI stripped, newline
/// excluded) and the contiguous `(byte-range, style)` runs that tile it.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ParsedLine {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
}

/// Run the colorizer `command`, feeding `input` on stdin and returning its
/// stdout. `None` if the command can't be spawned or exits non-zero.
pub(crate) fn run(_command: &[String], _input: &str) -> Option<String> {
    None
}

/// Parse ANSI-colored text into per-line styled runs.
pub(crate) fn parse_ansi_lines(_output: &str) -> Vec<ParsedLine> {
    vec![]
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
