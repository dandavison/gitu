//! Reading input up to a byte limit.
//!
//! gitu reads all of its input before it draws anything, so output it cannot get
//! to the end of quickly — `git log -p` over a whole history, most easily —
//! leaves it showing nothing for as long as that takes. Reading stops at
//! `general.max_input_bytes` instead. This is a limit on reading eagerly, not a
//! substitute for rendering incrementally: what is past the limit is dropped,
//! and the only thing that can be said about it is that it was there.
//!
//! What is kept is cut back to something the rest of gitu can work with, since a
//! hunk missing its last lines is one git will not apply.

use std::io::{self, Read};

/// Input read up to a limit, and whether the limit is what stopped it.
pub(crate) struct Capped {
    pub text: String,
    pub truncated: bool,
}

impl Capped {
    /// Read at most `limit` bytes of `reader`. The rest is left unread: a git
    /// process still writing it blocks there until gitu exits and closes the
    /// pipe, so it costs nothing but its own idle process.
    pub(crate) fn read(reader: impl Read, limit: u64) -> io::Result<Self> {
        let mut bytes = Vec::new();
        // One byte past the limit, so that stopping at it is distinguishable
        // from input that happens to be exactly that long.
        reader.take(limit + 1).read_to_end(&mut bytes)?;

        let truncated = bytes.len() as u64 > limit;
        let text = String::from_utf8_lossy(&bytes).into_owned();

        Ok(Self {
            text: if truncated {
                whole(&text).to_owned()
            } else {
                text
            },
            truncated,
        })
    }
}

/// `text` up to the start of the file diff or hunk it ends inside, or up to its
/// last whole line where it has no diff in it. The last hunk is dropped whether
/// or not it was in fact complete: the line counts in its header say how long it
/// should be, and re-parsing the patch to find out is work for one line of it.
fn whole(text: &str) -> &str {
    let (mut file, mut hunk, mut line) = (None, None, 0);
    let mut offset = 0;

    for row in text.split_inclusive('\n') {
        let content = past_escapes(row);
        if content.starts_with("diff --git ") {
            file = Some(offset);
        } else if content.starts_with("@@ ") {
            hunk = Some(offset);
        }

        offset += row.len();
        if row.ends_with('\n') {
            line = offset;
        }
    }

    match (file, hunk) {
        // A file diff whose last hunk is the one being dropped keeps its
        // earlier hunks; one with no whole hunk in it goes entirely.
        (Some(file), Some(hunk)) if hunk > file => &text[..hunk],
        (Some(file), _) => &text[..file],
        (None, Some(hunk)) => &text[..hunk],
        (None, None) => &text[..line],
    }
}

/// `row` with any leading SGR sequence dropped, so that its first character is
/// what says what it is: git colours what it writes to a pager, and both of the
/// headers looked for here arrive coloured.
fn past_escapes(row: &str) -> &str {
    let mut rest = row;
    while let Some(sequence) = rest.strip_prefix('\x1b') {
        let Some(end) = sequence.find(|c: char| c.is_ascii_alphabetic()) else {
            return rest;
        };
        rest = &sequence[end + 1..];
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/f.rs b/f.rs\n\
                         --- a/f.rs\n\
                         +++ b/f.rs\n\
                         @@ -1 +1 @@\n\
                         -one\n\
                         +two\n";

    fn read(text: &str, limit: u64) -> Capped {
        Capped::read(text.as_bytes(), limit).unwrap()
    }

    #[test]
    fn input_within_the_limit_arrives_whole() {
        let capped = read(PATCH, PATCH.len() as u64);
        assert_eq!(capped.text, PATCH);
        assert!(!capped.truncated);
    }

    #[test]
    fn a_file_diff_with_no_whole_hunk_in_it_is_dropped() {
        let text = format!("{PATCH}diff --git a/g.rs b/g.rs\n--- a/g");
        let capped = read(&text, (text.len() - 1) as u64);
        assert_eq!(capped.text, PATCH);
        assert!(capped.truncated);
    }

    #[test]
    fn a_hunk_cut_short_is_dropped_and_the_one_before_it_kept() {
        let text = format!("{PATCH}@@ -9 +9 @@\n-three\n+fo");
        assert_eq!(read(&text, (text.len() - 1) as u64).text, PATCH);
    }

    #[test]
    fn text_with_no_diff_in_it_is_cut_at_a_line() {
        assert_eq!(read("one\ntwo\nthr", 10).text, "one\ntwo\n");
    }

    /// The cut is by byte and a character may span the limit, so what is read is
    /// not necessarily valid UTF-8 even when the input is.
    #[test]
    fn a_character_split_by_the_limit_does_not_fail_the_read() {
        let text = "one\ntwö\n";
        let inside_the_character = (text.find('ö').unwrap() + 1) as u64;
        assert_eq!(read(text, inside_the_character).text, "one\n");
    }

    #[test]
    fn a_coloured_patch_is_cut_at_the_same_place() {
        let coloured = |text: &str| {
            text.lines()
                .map(|line| format!("\x1b[1m{line}\x1b[m\n"))
                .collect::<String>()
        };
        let patch = coloured(PATCH);
        let text = format!(
            "{patch}{}",
            coloured("diff --git a/g.rs b/g.rs\n--- a/g.rs")
        );

        assert_eq!(read(&text, (text.len() - 1) as u64).text, patch);
    }
}
