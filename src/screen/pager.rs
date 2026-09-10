//! The screen `gitu --pager` shows: a patch that arrived on stdin rather than
//! one gitu asked git for.
//!
//! Set as git's pager, gitu is handed the very bytes `git diff` / `git show`
//! produce, so the patch text is the source of truth exactly as it is
//! everywhere else — the difference is only where it came from. git colours
//! what it writes to a pager, so the text is stripped of ANSI before it is
//! parsed; the colours in the view are the renderer's, applied afterwards.

use super::Screen;
use crate::{
    Res,
    config::Config,
    git::diff::{Diff, DiffType},
    gitu_diff,
    items::{self, RenderParams},
};
use std::{rc::Rc, sync::Arc};

pub(crate) fn create(config: Arc<Config>, params: RenderParams, patch: String) -> Res<Screen> {
    let diff = Rc::new(parse(&patch));

    Screen::new(
        Arc::clone(&config),
        params,
        Box::new(move |params: RenderParams| {
            // Nothing parsed as a patch, so there is nothing to fold or stage:
            // show what arrived, as it arrived.
            if diff.file_diffs.is_empty() {
                return Ok(items::plain_rows(&patch));
            }

            Ok(items::create_diff_items(
                &config, &params, &diff, 0, false, None,
            ))
        }),
    )
}

/// Parse piped output into a diff. Anything that isn't a `diff --git` patch
/// yields no files, which is how "there is no structure here to offer" is said.
fn parse(patch: &str) -> Diff {
    let text = crate::diff_colorizer::strip_ansi(patch);
    let file_diffs = gitu_diff::Parser::new(&text)
        .parse_diff()
        .unwrap_or_default();

    Diff {
        text,
        diff_type: DiffType::TreeToTree,
        file_diffs,
        commit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::item_data::ItemData;

    const PATCH: &str = "diff --git a/f.rs b/f.rs\n\
                         index 1..2 100644\n\
                         --- a/f.rs\n\
                         +++ b/f.rs\n\
                         @@ -1,3 +1,3 @@\n\
                         \x20keep\n\
                         -gone\n\
                         +added\n";

    fn items_of(patch: &str) -> Vec<ItemData> {
        let diff = Rc::new(parse(patch));
        let config = config::init_test_config().unwrap();
        items::create_diff_items(&config, &Default::default(), &diff, 0, false, None)
            .into_iter()
            .map(|item| item.data)
            .collect()
    }

    #[test]
    fn a_piped_patch_becomes_a_file_a_hunk_and_its_lines() {
        let items = items_of(PATCH);

        assert!(
            matches!(items.first(), Some(ItemData::Delta { .. })),
            "got {items:?}"
        );
        assert!(items.iter().any(|d| matches!(d, ItemData::Hunk { .. })));
        assert_eq!(
            items
                .iter()
                .filter(|d| matches!(d, ItemData::HunkLine { .. }))
                .count(),
            3
        );
    }

    /// git colours what it writes to its pager, and the parser reads none of it
    /// through the escapes, so they have to come off first.
    #[test]
    fn a_coloured_patch_parses_the_same_as_a_plain_one() {
        let coloured = "\x1b[1mdiff --git a/f.rs b/f.rs\x1b[m\n\
             \x1b[1mindex 1..2 100644\x1b[m\n\
             \x1b[1m--- a/f.rs\x1b[m\n\
             \x1b[1m+++ b/f.rs\x1b[m\n\
             \x1b[36m@@ -1,3 +1,3 @@\x1b[m\n\
             \x20keep\n\
             \x1b[31m-gone\x1b[m\n\
             \x1b[32m+added\x1b[m\n";

        assert_eq!(items_of(coloured).len(), items_of(PATCH).len());
    }

    /// Input that is not a git patch offers no structure rather than failing.
    #[test]
    fn output_that_is_not_a_patch_yields_no_files() {
        assert!(items_of("this is not a diff\nnor is this\n").is_empty());
        assert!(items_of("").is_empty());
    }

    /// ... and is shown as it arrived, rather than as an empty screen.
    #[test]
    fn output_that_is_not_a_patch_is_still_readable() {
        let grep = "m.rs:2:    let alpha = 1;\nm.rs:3:    let gamma = 42;\n";
        let screen = create(
            Arc::new(config::init_test_config().unwrap()),
            RenderParams {
                size: ratatui::layout::Size::new(80, 20),
                features: Rc::from([]),
            },
            grep.to_string(),
        )
        .unwrap();

        assert_eq!(
            screen.row_texts(),
            ["m.rs:2:    let alpha = 1;", "m.rs:3:    let gamma = 42;"]
        );
    }
}
