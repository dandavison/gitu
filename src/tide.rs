//! Integration with the `tide` terminal surface. When gitu runs as tide's git
//! view, `$TIDE_STATE` names a file to which we write the next-view directive
//! before quitting, so the dispatcher can switch views.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub(crate) fn state_file() -> Option<String> {
    std::env::var("TIDE_STATE").ok().filter(|s| !s.is_empty())
}

pub(crate) fn emit(directive: &str) {
    if let Some(path) = state_file() {
        let _ = std::fs::write(path, directive);
    }
}

/// The view a key requests when running under tide, if any. `Alt-d` (git) is
/// omitted: it is the current view.
pub(crate) fn switch_view(key: &KeyEvent) -> Option<&'static str> {
    if state_file().is_none()
        || key.kind != KeyEventKind::Press
        || !key.modifiers.contains(KeyModifiers::ALT)
    {
        return None;
    }
    match key.code {
        KeyCode::Char('f') => Some("files"),
        KeyCode::Char('j') => Some("search"),
        KeyCode::Char('k') => Some("edit"),
        _ => None,
    }
}
