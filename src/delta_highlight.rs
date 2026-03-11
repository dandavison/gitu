use crate::config::DeltaConfig;
use crate::git::diff::Diff;
use crate::highlight::HunkHighlights;
use ansi_to_tui::IntoText;
use cached::{SizedCache, proc_macro::cached};
use ratatui::style::Style;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::ops::Range;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;

struct DeltaDiffHighlights {
    /// Per-line highlight spans. Range is byte offset within stripped line.
    lines: Vec<Vec<(Range<usize>, Style)>>,
    /// Byte offset of each line start in the original diff text.
    line_starts: Vec<usize>,
}

pub(crate) fn highlight_hunk_with_delta(
    config: &DeltaConfig,
    diff: &Rc<Diff>,
    file_i: usize,
    hunk_i: usize,
) -> Arc<HunkHighlights> {
    let mut hasher = DefaultHasher::new();
    diff.text.hash(&mut hasher);
    let diff_hash = hasher.finish();

    let cached = delta_highlight_diff(diff_hash, config, diff);

    let hunk = &diff.file_diffs[file_i].hunks[hunk_i];
    let content_range = &hunk.content.range;

    let first_line = cached
        .line_starts
        .partition_point(|&offset| offset < content_range.start);
    let end_line = cached
        .line_starts
        .partition_point(|&offset| offset < content_range.end);

    let mut highlights = HunkHighlights::new();

    for line_idx in first_line..end_line {
        let start = highlights.spans_len();
        for (range, style) in &cached.lines[line_idx] {
            highlights.push_span(range.clone(), *style);
        }
        highlights.push_line_index(start);
    }

    Arc::new(highlights)
}

#[cached(
    ty = "SizedCache<u64, Arc<DeltaDiffHighlights>>",
    create = "{ SizedCache::with_size(50) }",
    convert = r#"{ _diff_hash }"#
)]
fn delta_highlight_diff(
    _diff_hash: u64,
    config: &DeltaConfig,
    diff: &Rc<Diff>,
) -> Arc<DeltaDiffHighlights> {
    match run_delta(config, &diff.text) {
        Ok(highlights) => Arc::new(highlights),
        Err(e) => {
            log::warn!("delta highlighting failed, using empty highlights: {e}");
            Arc::new(build_highlights_from_text(&diff.text))
        }
    }
}

fn run_delta(config: &DeltaConfig, diff_text: &str) -> Result<DeltaDiffHighlights, String> {
    let mut child = Command::new(&config.path)
        .args(["--color-only", "--paging=never", "--no-gitconfig"])
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn delta: {e}"))?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(diff_text.as_bytes())
        .map_err(|e| format!("failed to write to delta stdin: {e}"))?;

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for delta: {e}"))?;

    if !output.status.success() {
        return Err(format!("delta exited with status {}", output.status));
    }

    let line_starts = compute_line_starts(diff_text);
    let lines = parse_ansi_lines(&output.stdout, diff_text, &line_starts);

    Ok(DeltaDiffHighlights { lines, line_starts })
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' && i + 1 < text.len() {
            starts.push(i + 1);
        }
    }
    starts
}

fn parse_ansi_lines(
    ansi_output: &[u8],
    diff_text: &str,
    line_starts: &[usize],
) -> Vec<Vec<(Range<usize>, Style)>> {
    let text = match ansi_output.into_text() {
        Ok(t) => t,
        Err(e) => {
            log::warn!("failed to parse delta ANSI output: {e}");
            return line_starts.iter().map(|_| vec![]).collect();
        }
    };

    let diff_lines: Vec<&str> = diff_text.split('\n').collect();

    text.lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let plain_line = diff_lines.get(i).unwrap_or(&"");
            ansi_line_to_spans(line, plain_line)
        })
        .collect()
}

fn ansi_line_to_spans(
    line: ratatui::text::Line<'_>,
    plain_line: &str,
) -> Vec<(Range<usize>, Style)> {
    let mut spans = Vec::new();
    let mut byte_offset = 0usize;

    for span in line.spans {
        let span_text = span.content.as_ref();
        // Map back to the plain line by searching from current offset.
        // delta's --color-only preserves text content, so span text should
        // match the plain line at the expected position.
        if let Some(pos) = find_substr(plain_line, byte_offset, span_text) {
            if span.style != Style::default() {
                spans.push((pos..pos + span_text.len(), span.style));
            }
            byte_offset = pos + span_text.len();
        } else {
            // Content mismatch; use current offset as best effort.
            let end = (byte_offset + span_text.len()).min(plain_line.len());
            if span.style != Style::default() && byte_offset < end {
                spans.push((byte_offset..end, span.style));
            }
            byte_offset = end;
        }
    }

    // Trim trailing \r\n ranges (visual line end, matching line_range_iterator behavior).
    let visual_end = if plain_line.ends_with("\r\n") {
        plain_line.len().saturating_sub(2)
    } else {
        plain_line.len()
    };

    spans.retain(|(range, _)| range.start < visual_end);
    for (range, _) in &mut spans {
        if range.end > visual_end {
            range.end = visual_end;
        }
    }

    spans
}

fn find_substr(haystack: &str, from: usize, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    haystack[from..].find(needle).map(|i| i + from)
}

/// Fallback: produce empty highlights for each line.
fn build_highlights_from_text(text: &str) -> DeltaDiffHighlights {
    let line_starts = compute_line_starts(text);
    let lines = line_starts.iter().map(|_| vec![]).collect();
    DeltaDiffHighlights { lines, line_starts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn test_ansi_line_to_spans() {
        // \x1b[31m-old\x1b[0m => red "-old"
        let ansi_bytes = b"\x1b[31m-old\x1b[0m";
        let text = ansi_bytes.into_text().unwrap();
        let line = text.lines.into_iter().next().unwrap();
        let spans = ansi_line_to_spans(line, "-old");

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].0, 0..4);
        assert_eq!(spans[0].1.fg, Some(Color::Red));
    }

    #[test]
    fn test_ansi_line_multiple_spans() {
        // Red "ab" then green "cd"
        let ansi_bytes = b"\x1b[31mab\x1b[32mcd\x1b[0m";
        let text = ansi_bytes.into_text().unwrap();
        let line = text.lines.into_iter().next().unwrap();
        let spans = ansi_line_to_spans(line, "abcd");

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0, 0..2);
        assert_eq!(spans[0].1.fg, Some(Color::Red));
        assert_eq!(spans[1].0, 2..4);
        assert_eq!(spans[1].1.fg, Some(Color::Green));
    }

    #[test]
    fn test_line_starts_computation() {
        let text = "line1\nline2\nline3\n";
        let starts = compute_line_starts(text);
        assert_eq!(starts, vec![0, 6, 12]);
        // Note: no start for trailing empty after final \n since i+1 == len
    }

    #[test]
    fn test_line_starts_single_line() {
        let starts = compute_line_starts("hello");
        assert_eq!(starts, vec![0]);
    }

    #[test]
    fn test_line_starts_empty() {
        let starts = compute_line_starts("");
        assert_eq!(starts, vec![0]);
    }

    #[test]
    fn test_hunk_line_mapping() {
        // Simulate a diff with known structure:
        // "header\n" (line 0, bytes 0..7)
        // "+added\n" (line 1, bytes 7..14)
        // " ctx\n"   (line 2, bytes 14..19)
        let diff_text = "header\n+added\n ctx\n";
        let line_starts = compute_line_starts(diff_text);
        assert_eq!(line_starts, vec![0, 7, 14]);

        // If hunk content starts at byte 7 ("+added\n ctx\n"):
        let content_start = 7;
        let content_end = 19;

        let first_line = line_starts.partition_point(|&o| o < content_start);
        let end_line = line_starts.partition_point(|&o| o < content_end);

        assert_eq!(first_line, 1);
        assert_eq!(end_line, 3);
        // Lines 1..3 (indices 1 and 2) are the hunk content lines.
    }
}
