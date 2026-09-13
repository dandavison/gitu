use crate::Res;
use crate::error::Error;
use crate::text_input::TextInput;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::borrow::Cow;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

pub(crate) struct PromptData {
    pub(crate) prompt_text: Cow<'static, str>,
}

#[derive(Clone, Copy)]
pub(crate) enum HistoryKind {
    GitCommand,
    FilePatterns,
}

pub(crate) struct Prompt {
    pub(crate) data: Option<PromptData>,
    pub(crate) state: TextInput,
    /// What the last kill took, for `ctrl+y` to put back.
    killed: String,
    history_dir: Option<PathBuf>,
    active_history: Option<ActiveHistory>,
}

struct ActiveHistory {
    kind: HistoryKind,
    entries: Vec<String>,
    position: usize,
    draft: String,
}

impl Prompt {
    pub(crate) fn new() -> Self {
        Prompt {
            data: None,
            state: TextInput::default(),
            killed: String::new(),
            history_dir: None,
            active_history: None,
        }
    }

    pub(crate) fn with_history(history_dir: PathBuf) -> Self {
        Self {
            history_dir: Some(history_dir),
            ..Self::new()
        }
    }

    pub(crate) fn set(&mut self, data: PromptData) {
        self.data = Some(data);
        self.state.focused = true;
    }

    pub(crate) fn reset(&mut self) {
        self.data = None;
        self.state = TextInput::default();
        self.active_history = None;
    }

    pub(crate) fn start_history(&mut self, kind: Option<HistoryKind>) -> Res<()> {
        let (Some(kind), Some(dir)) = (kind, &self.history_dir) else {
            return Ok(());
        };
        let draft = self.state.value.as_str().to_owned();
        let entries = read_history(dir, kind)?
            .into_iter()
            .filter(|entry| entry != &draft)
            .collect::<Vec<_>>();
        self.active_history = Some(ActiveHistory {
            kind,
            position: entries.len(),
            entries,
            draft,
        });
        Ok(())
    }

    pub(crate) fn remember(&self, value: &str) -> Res<()> {
        let (Some(history), Some(dir)) = (&self.active_history, &self.history_dir) else {
            return Ok(());
        };
        if value.is_empty() {
            return Ok(());
        }

        fs::create_dir_all(dir).map_err(Error::WritePromptHistory)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(history_path(dir, history.kind))
            .map_err(Error::WritePromptHistory)?;
        file.write_all(format!("{value}\n").as_bytes())
            .map_err(Error::WritePromptHistory)
    }

    /// A line being edited is a line to edit as any other: the readline keys,
    /// over and above the single-character motions and deletions
    /// [`TextInput`] already binds.
    ///
    /// Words come in the two kinds readline has, and the distinction earns its
    /// keep on the commands gitu puts here: `alt+b`/`alt+f`/`alt+d` step over
    /// runs of letters and digits, so they move within a path, while `ctrl+w`
    /// takes back a whole whitespace-delimited token, which is what a pathspec
    /// is.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }

        const ALT: KeyModifiers = KeyModifiers::ALT;
        const CTRL: KeyModifiers = KeyModifiers::CONTROL;
        let cursor = self.cursor();

        match (key.code, key.modifiers) {
            (KeyCode::Up, KeyModifiers::NONE) => self.history_previous(),
            (KeyCode::Down, KeyModifiers::NONE) => self.history_next(),
            (KeyCode::Char('b'), ALT) | (KeyCode::Left, ALT | CTRL) => {
                self.move_to(self.previous_word_start());
            }
            (KeyCode::Char('f'), ALT) | (KeyCode::Right, ALT | CTRL) => {
                self.move_to(self.next_word_end());
            }
            (KeyCode::Char('w'), CTRL) => self.kill(self.previous_token_start()..cursor),
            (KeyCode::Backspace, ALT) => self.kill(self.previous_word_start()..cursor),
            (KeyCode::Char('d'), ALT) => self.kill(cursor..self.next_word_end()),
            (KeyCode::Char('k'), CTRL) => self.kill(cursor..self.len()),
            (KeyCode::Char('u'), CTRL) => self.kill(0..self.len()),
            (KeyCode::Char('y'), CTRL) => self.yank(),
            (KeyCode::Char('t'), CTRL) => self.transpose(),
            _ => self.state.handle_key_event(key),
        }
    }

    fn cursor(&self) -> usize {
        self.state.position().min(self.len())
    }

    fn len(&self) -> usize {
        self.state.value.as_str().chars().count()
    }

    fn chars(&self) -> Vec<char> {
        self.state.value.as_str().chars().collect()
    }

    fn move_to(&mut self, cursor: usize) {
        self.state.move_to(cursor);
    }

    fn history_previous(&mut self) {
        let Some(history) = &mut self.active_history else {
            return;
        };
        save_history_edit(history, self.state.value.as_str());
        if history.position > 0 {
            history.position -= 1;
        }
        self.show_history_value();
    }

    fn history_next(&mut self) {
        let Some(history) = &mut self.active_history else {
            return;
        };
        save_history_edit(history, self.state.value.as_str());
        if history.position < history.entries.len() {
            history.position += 1;
        }
        self.show_history_value();
    }

    fn show_history_value(&mut self) {
        let Some(history) = &self.active_history else {
            return;
        };
        let value = history
            .entries
            .get(history.position)
            .unwrap_or(&history.draft);
        self.state.value = value.clone();
        self.state.move_to(usize::MAX);
    }

    /// Take `range` out of the line, keeping it for [`Self::yank`]. A kill of
    /// nothing is not a kill: it leaves what was killed before to be put back.
    fn kill(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }

        let chars = self.chars();
        self.killed = chars[range.clone()].iter().collect();
        self.state.value = chars[..range.start]
            .iter()
            .chain(&chars[range.end..])
            .collect();
        self.move_to(range.start);
    }

    fn yank(&mut self) {
        let chars = self.chars();
        let cursor = self.cursor();

        self.state.value = chars[..cursor]
            .iter()
            .chain(self.killed.chars().collect::<Vec<_>>().iter())
            .chain(&chars[cursor..])
            .collect();
        self.move_to(cursor + self.killed.chars().count());
    }

    /// Swap the two characters the cursor sits between, and step over them. At
    /// the end of the line there is nothing to the right to swap with, so the
    /// last two change places instead.
    fn transpose(&mut self) {
        let mut chars = self.chars();
        let cursor = self.cursor().min(chars.len().saturating_sub(1)).max(1);
        if chars.len() < 2 {
            return;
        }

        chars.swap(cursor - 1, cursor);
        self.state.value = chars.iter().collect();
        self.move_to((cursor + 1).min(self.len()));
    }

    /// Where the run of letters and digits before the cursor begins.
    fn previous_word_start(&self) -> usize {
        let chars = self.chars();
        let end = skipping_back(&chars, self.cursor(), |c| !c.is_alphanumeric());
        skipping_back(&chars, end, |c| c.is_alphanumeric())
    }

    /// Where the run of letters and digits after the cursor ends.
    fn next_word_end(&self) -> usize {
        let chars = self.chars();
        let start = skipping_on(&chars, self.cursor(), |c| !c.is_alphanumeric());
        skipping_on(&chars, start, |c| c.is_alphanumeric())
    }

    /// Where the whitespace-delimited token before the cursor begins.
    fn previous_token_start(&self) -> usize {
        let chars = self.chars();
        let end = skipping_back(&chars, self.cursor(), |c| c.is_whitespace());
        skipping_back(&chars, end, |c| !c.is_whitespace())
    }
}

fn save_history_edit(history: &mut ActiveHistory, value: &str) {
    if let Some(entry) = history.entries.get_mut(history.position) {
        *entry = value.to_owned();
    } else {
        history.draft = value.to_owned();
    }
}

fn read_history(dir: &Path, kind: HistoryKind) -> Res<Vec<String>> {
    match fs::read_to_string(history_path(dir, kind)) {
        Ok(contents) => Ok(contents.lines().map(str::to_owned).collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(Error::ReadPromptHistory(err)),
    }
}

fn history_path(dir: &Path, kind: HistoryKind) -> PathBuf {
    dir.join(match kind {
        HistoryKind::GitCommand => "git-command-history",
        HistoryKind::FilePatterns => "file-pattern-history",
    })
}

fn skipping_back(chars: &[char], from: usize, matching: impl Fn(&char) -> bool) -> usize {
    let kept = chars[..from]
        .iter()
        .rev()
        .take_while(|c| matching(c))
        .count();
    from - kept
}

fn skipping_on(chars: &[char], from: usize, matching: impl Fn(&char) -> bool) -> usize {
    from + chars[from..].iter().take_while(|c| matching(c)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_parser::parse_test_keys;

    /// The line and cursor after typing `keys`, both written with `|` where
    /// the cursor is.
    fn edited(before: &str, keys: &str) -> String {
        let (value, rest) = before.split_once('|').expect("no cursor in the line");
        let mut prompt = Prompt::new();
        prompt.state.value = format!("{value}{rest}");
        prompt.move_to(value.chars().count());

        let ("", keys) = parse_test_keys(keys).unwrap() else {
            unreachable!()
        };
        for (modifiers, code) in keys {
            prompt.handle_key(KeyEvent::new(code, modifiers));
        }

        let chars = prompt.chars();
        let cursor = prompt.cursor();
        format!(
            "{}|{}",
            chars[..cursor].iter().collect::<String>(),
            chars[cursor..].iter().collect::<String>()
        )
    }

    /// A word is a run of letters and digits, so a motion stops inside a path
    /// rather than stepping over the whole of it.
    #[test]
    fn a_word_motion_steps_over_letters_and_digits() {
        assert_eq!(edited("git diff main|", "<alt+b>"), "git diff |main");
        assert_eq!(edited("git diff main|", "<alt+b><alt+b>"), "git |diff main");
        assert_eq!(edited("|src/lib.rs", "<alt+f>"), "src|/lib.rs");
        assert_eq!(edited("|src/lib.rs", "<alt+f><alt+f>"), "src/lib|.rs");
        assert_eq!(edited("|a", "<alt+b>"), "|a");
        assert_eq!(edited("a|", "<alt+f>"), "a|");
        assert_eq!(edited("a b|", "<ctrl+left>"), "a |b");
        assert_eq!(edited("|a b", "<ctrl+right>"), "a| b");
    }

    /// `ctrl+w` takes back a whole token, which is what a pathspec is: nobody
    /// wants it in the pieces its punctuation makes.
    #[test]
    fn ctrl_w_takes_back_a_whole_token() {
        assert_eq!(
            edited("git diff -- ':(top,exclude)*_test.go'|", "<ctrl+w>"),
            "git diff -- |"
        );
        assert_eq!(edited("git diff  |", "<ctrl+w>"), "git |");
        assert_eq!(edited("|", "<ctrl+w>"), "|");
    }

    #[test]
    fn a_word_can_be_killed_either_side_of_the_cursor() {
        assert_eq!(edited("src/lib.rs|", "<alt+backspace>"), "src/lib.|");
        assert_eq!(edited("|src/lib.rs", "<alt+d>"), "|/lib.rs");
        assert_eq!(edited("a |b c", "<alt+d>"), "a | c");
    }

    /// What a kill took, `ctrl+y` puts back — which is how a token is moved,
    /// and how a mistaken kill is undone.
    #[test]
    fn what_was_killed_can_be_put_back() {
        assert_eq!(
            edited("git diff main|", "<ctrl+w><ctrl+y>"),
            "git diff main|"
        );
        assert_eq!(
            edited("git diff main|", "<ctrl+w><ctrl+a><ctrl+y>"),
            "main|git diff "
        );
        // A kill of nothing leaves the last kill to be put back.
        assert_eq!(edited("a b|", "<ctrl+w><ctrl+a><ctrl+w><ctrl+y>"), "b|a ");
    }

    #[test]
    fn a_line_can_be_killed_to_the_end_or_entirely() {
        assert_eq!(edited("git |diff main", "<ctrl+k>"), "git |");
        assert_eq!(
            edited("git |diff main", "<ctrl+k><ctrl+y>"),
            "git diff main|"
        );
        assert_eq!(edited("git |diff", "<ctrl+u>"), "|");
        assert_eq!(edited("git |diff", "<ctrl+u><ctrl+y>"), "git diff|");
    }

    #[test]
    fn two_characters_change_places() {
        assert_eq!(edited("ab|cd", "<ctrl+t>"), "acb|d");
        assert_eq!(edited("ab|", "<ctrl+t>"), "ba|");
        assert_eq!(edited("|ab", "<ctrl+t>"), "ba|");
        assert_eq!(edited("a|", "<ctrl+t>"), "a|");
    }

    /// The keys `TextInput` binds still do what they did.
    #[test]
    fn the_single_character_keys_are_untouched() {
        assert_eq!(edited("git dif|", "f"), "git diff|");
        assert_eq!(edited("git diff|", "<backspace>"), "git dif|");
        assert_eq!(edited("git |diff", "<ctrl+e>"), "git diff|");
        assert_eq!(edited("git diff|", "<ctrl+a>"), "|git diff");
        assert_eq!(edited("git diff|", "<left>"), "git dif|f");
    }

    /// Characters are characters, not bytes: a motion over them lands between
    /// them.
    #[test]
    fn a_multibyte_line_edits_by_character() {
        assert_eq!(edited("héllo wörld|", "<ctrl+w>"), "héllo |");
        assert_eq!(edited("héllo wörld|", "<alt+b>"), "héllo |wörld");
        assert_eq!(edited("héllo|", "<backspace>"), "héll|");
    }
}
