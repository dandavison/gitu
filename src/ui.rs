use std::borrow::Cow;

use crate::app::State;
use crate::screen;
use crate::ui::layout::LayoutItem;
use itertools::Itertools;
use layout::LayoutTree;
use layout::OPTS;
use ratatui::Frame;
use ratatui::prelude::*;
use std::num::NonZeroU16;
use tui_prompts::State as _;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) mod layout;
mod menu;
pub mod picker;

const CARET: &str = "\u{2588}";
const DASHES: &str = "────────────────────────────────────────────────────────────────";

pub(crate) type UiTree<'a> = LayoutTree<(Cow<'a, str>, Style)>;

pub(crate) fn ui(frame: &mut Frame, state: &mut State) {
    let size = frame.area().as_size();
    // The screen gets what the panels below it leave, and must know it: it
    // scrolls to keep its cursor within the rows it is actually given.
    let screen_size = Size::new(
        size.width,
        size.height.saturating_sub(panels_height(state, size)),
    );
    let mut layout = UiTree::new();

    layout.vertical(None, OPTS, |layout| {
        layout.vertical(None, OPTS.grow(), |layout| {
            let hide_cursor = state.picker.is_some();
            screen::layout_screen(
                layout,
                screen_size,
                state.screens.last().unwrap(),
                hide_cursor,
            );
        });

        layout.vertical(None, OPTS, |layout| layout_panels(layout, state, size));
    });

    layout.compute([frame.area().width, frame.area().height]);

    for item in layout.iter() {
        let LayoutItem { data, pos, size } = item;
        let area = Rect::new(pos[0], pos[1], size[0], size[1]);
        let (text, style) = data;
        if let Some((uri, text)) = osc8_parts(text) {
            let text = Cow::Borrowed(text);
            frame.render_widget(SpanRef(&text, *style, Some(uri)), area);
        } else {
            frame.render_widget(SpanRef(text, *style, None), area);
        }
    }

    layout.clear();

    state.screens.last_mut().unwrap().size = screen_size;
}

/// The menu, command log, prompt and picker, which sit below the screen.
fn layout_panels<'a>(layout: &mut UiTree<'a>, state: &'a State, size: Size) {
    menu::layout_menu(layout, state, size.width as usize);
    layout_command_log(layout, state, size.width as usize);
    layout_prompt(layout, state, size.width as usize);
    layout_picker(layout, state, size.width as usize);
    if !state.pending_keys.is_empty() {
        let keys = &state
            .pending_keys
            .iter()
            .map(|(_, k)| k.to_string())
            .collect::<String>();

        layout_span(layout, (("    ".to_string() + keys).into(), Style::new()));
    }
}

/// How many rows the panels take, by laying them out on their own.
fn panels_height(state: &State, size: Size) -> u16 {
    let mut layout = UiTree::new();
    layout.vertical(None, OPTS, |layout| layout_panels(layout, state, size));
    layout.compute([size.width, size.height]);

    layout
        .iter()
        .map(|item| item.pos[1] + item.size[1])
        .max()
        .unwrap_or(0)
}

struct SpanRef<'a>(&'a Cow<'a, str>, Style, Option<&'a str>);

impl<'a> Widget for SpanRef<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let SpanRef(text, style, hyperlink) = self;
        let Some(hyperlink) = hyperlink else {
            buf.set_string(area.x, area.y, text, style);
            return;
        };
        for (offset, grapheme) in text.graphemes(true).take(area.width as usize).enumerate() {
            let Some(cell) = buf.cell_mut(Position::new(area.x + offset as u16, area.y)) else {
                break;
            };
            cell.set_symbol(&osc8_hyperlink(grapheme, hyperlink));
            cell.set_style(style);
            cell.set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
                NonZeroU16::MIN,
            ));
        }
    }
}

fn layout_command_log<'a>(layout: &mut UiTree<'a>, state: &State, width: usize) {
    if !state.current_cmd_log.is_empty() {
        let separator_style = Style::from(&state.config.style.separator);
        repeat_chars(layout, width, DASHES, separator_style);
        layout_text(layout, state.current_cmd_log.format_log(&state.config));
    }
}

fn layout_prompt<'a>(layout: &mut UiTree<'a>, state: &'a State, width: usize) {
    let Some(ref prompt_data) = state.prompt.data else {
        return;
    };

    let prompt_symbol = state.prompt.state.status().symbol();
    let separator_style = Style::from(&state.config.style.separator);
    let prompt_style = Style::from(&state.config.style.prompt);

    repeat_chars(layout, width, DASHES, separator_style);
    layout.horizontal(None, OPTS, |layout| {
        // A prompt that says nothing is drawn as nothing but the line typed on.
        if !prompt_data.prompt_text.is_empty() {
            layout_span(layout, (prompt_symbol.content, prompt_symbol.style));
            layout_span(layout, (" ".into(), Style::new()));
            layout_span(
                layout,
                (prompt_data.prompt_text.as_ref().into(), prompt_style),
            );
            layout_span(layout, (" › ".into(), prompt_style));
        }
        layout_typed_line(
            layout,
            state.prompt.state.value(),
            state.prompt.state.position(),
        );
    });
}

/// A line being typed, with the cursor where the next character will land: on
/// the character it is at, which is drawn through it, or standing on its own at
/// the end of the line where there is no character to draw.
pub(crate) fn layout_typed_line<'a>(layout: &mut UiTree<'a>, value: &str, position: usize) {
    let mut rest = value.chars();
    let before: String = rest.by_ref().take(position).collect();
    let at = rest.next();
    let after: String = rest.collect();

    layout_span(layout, (before.into(), Style::new()));
    match at {
        Some(character) => {
            layout_span(
                layout,
                (character.to_string().into(), Style::new().reversed()),
            );
            layout_span(layout, (after.into(), Style::new()));
        }
        None => layout_span(layout, (CARET.into(), Style::new())),
    }
}

fn layout_picker<'a>(layout: &mut UiTree<'a>, state: &'a State, width: usize) {
    if let Some(ref picker_state) = state.picker {
        picker::layout_picker(layout, picker_state, &state.config, width);
    }
}

pub(crate) fn layout_text<'a>(layout: &mut UiTree<'a>, text: Text<'a>) {
    layout.vertical(None, OPTS, |layout| {
        for line in text {
            layout_line(layout, line);
        }
    });
}

pub(crate) fn layout_line<'a>(layout: &mut UiTree<'a>, line: Line<'a>) {
    let line_style = line.style;
    layout.horizontal(None, OPTS, |layout| {
        for span in line {
            // Merge line.style with span.style
            let merged_style = line_style.patch(span.style);
            layout_span(layout, (span.content, merged_style));
        }
    });
}

pub(crate) fn layout_span<'a>(layout: &mut UiTree<'a>, span: (Cow<'a, str>, Style)) {
    let width = display_text(&span.0).graphemes(true).count() as u16;
    layout.leaf_with_size(span, [width, 1]);
}

pub(crate) fn display_text(text: &str) -> &str {
    osc8_parts(text).map_or(text, |(_, text)| text)
}

pub(crate) fn truncate_span(text: &str, graphemes: usize) -> String {
    let truncated = display_text(text)
        .graphemes(true)
        .take(graphemes)
        .collect::<String>();
    osc8_parts(text).map_or_else(
        || truncated.clone(),
        |(uri, _)| osc8_hyperlink(&truncated, uri),
    )
}

pub(crate) fn osc8_hyperlink(text: &str, uri: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\")
}

fn osc8_parts(text: &str) -> Option<(&str, &str)> {
    let (uri, text) = text.strip_prefix("\x1b]8;;")?.split_once("\x1b\\")?;
    Some((uri, text.strip_suffix("\x1b]8;;\x1b\\")?))
}

pub(crate) fn repeat_chars(layout: &mut UiTree, count: usize, chars: &'static str, style: Style) {
    let grapheme_count = chars.grapheme_indices(true).count();
    let full = count / grapheme_count;
    let partial = count % grapheme_count;

    layout.horizontal(None, OPTS, |layout| {
        for _ in 0..full {
            layout_span(layout, (chars.into(), style));
        }

        if partial > 0 {
            let end = chars
                .grapheme_indices(true)
                .tuple_windows()
                .take(partial)
                .last()
                .map(|((_, _), (end, _))| end)
                .unwrap_or(chars.len());

            layout_span(layout, (chars[..end].into(), style));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::CellDiffOption;
    use std::num::NonZeroU16;

    #[test]
    fn hyperlink_span_is_independently_redrawable_cells() {
        let text = Cow::Borrowed("link");
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));

        SpanRef(&text, Style::new(), Some("https://example.com"))
            .render(Rect::new(0, 0, 4, 1), &mut buffer);

        for (x, grapheme) in "link".chars().enumerate() {
            assert_eq!(
                buffer[(x as u16, 0)].symbol(),
                osc8_hyperlink(&grapheme.to_string(), "https://example.com")
            );
            assert_eq!(
                buffer[(x as u16, 0)].diff_option,
                CellDiffOption::ForcedWidth(NonZeroU16::MIN)
            );
        }
    }

    #[test]
    fn truncating_a_hyperlink_keeps_it_clickable() {
        let link = osc8_hyperlink("linked", "https://example.com");

        assert_eq!(
            truncate_span(&link, 4),
            osc8_hyperlink("link", "https://example.com")
        );
    }
}
