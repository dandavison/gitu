use crate::config::StyleConfig;
use crate::ui::layout::OPTS;
use crate::ui::{UiTree, layout_span};
use crate::{item_data::ItemData, ui};
use ratatui::{layout::Size, style::Style, text::Line};
use unicode_segmentation::UnicodeSegmentation;

use crate::{Res, config::Config, items::hash};

use super::Item;
use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

pub(crate) mod blame;
pub(crate) mod log;
pub(crate) mod rebase_todo;
pub(crate) mod show;
pub(crate) mod show_refs;
pub(crate) mod show_stash;
pub(crate) mod status;

const BOTTOM_CONTEXT_LINES: usize = 2;

#[derive(Copy, Clone, Debug)]
pub(crate) enum NavMode {
    Normal,
    Siblings { depth: usize },
    IncludeSubLines,
}

pub(crate) struct Screen {
    pub(crate) size: Size,
    /// The keymap this screen imposes while it is on top, if it isn't the
    /// ordinary one (the interactive rebase todo has its own single-key actions).
    pub(crate) menu: Option<crate::menu::Menu>,
    /// Whether that keymap is listed on screen. It stays up as long as the
    /// screen does, so it starts out of the way.
    pub(crate) show_menu: bool,
    cursor: usize,
    /// Where a multi-line selection was started, if one is being made. The
    /// selection runs from here to the cursor, inclusive.
    anchor: Option<usize>,
    scroll: usize,
    config: Arc<Config>,
    refresh_items: Box<dyn Fn(Size) -> Res<Vec<Item>>>,
    items: Vec<Item>,
    line_index: Vec<usize>,
    collapsed: HashSet<u64>,
}

impl Screen {
    pub(crate) fn new(
        config: Arc<Config>,
        size: Size,
        refresh_items: Box<dyn Fn(Size) -> Res<Vec<Item>>>,
    ) -> Res<Self> {
        let collapsed = config
            .general
            .collapsed_sections
            .clone()
            .into_iter()
            .map(hash)
            .collect();

        let mut screen = Self {
            cursor: 0,
            anchor: None,
            menu: None,
            show_menu: false,
            scroll: 0,
            size,
            config,
            refresh_items,
            items: vec![],
            line_index: vec![],
            collapsed,
        };

        screen.update()?;

        // TODO Maybe this should be done on update. Better keep track of toggled sections rather than collapsed then.
        screen
            .items
            .iter()
            .filter(|item| item.default_collapsed)
            .for_each(|item| {
                screen.collapsed.insert(item.id);
            });
        screen.update_line_index();

        screen.cursor = screen
            .find_first_hunk()
            .or_else(|| screen.find_first_selectable())
            .unwrap_or(0);

        Ok(screen)
    }

    fn find_first_hunk(&mut self) -> Option<usize> {
        (0..self.line_index.len()).find(|&line_i| {
            !self.at_line(line_i).unselectable
                && matches!(self.at_line(line_i).data, ItemData::Hunk { .. })
        })
    }

    fn find_first_selectable(&mut self) -> Option<usize> {
        (0..self.line_index.len()).find(|&line_i| !self.at_line(line_i).unselectable)
    }

    fn at_line(&self, line_i: usize) -> &Item {
        &self.items[self.line_index[line_i]]
    }

    pub(crate) fn select_next(&mut self, nav_mode: NavMode) {
        self.anchor = None;
        self.move_next(nav_mode);
    }

    fn move_next(&mut self, nav_mode: NavMode) {
        self.cursor = self.find_next(nav_mode);
        self.scroll_fit_end();
        self.scroll_fit_start();
    }

    fn scroll_fit_start(&mut self) {
        if self.items.is_empty() {
            return;
        }

        let top = self.cursor.saturating_sub(self.get_selected_item().depth);
        if top < self.scroll {
            self.scroll = top;
        }
    }

    fn scroll_fit_end(&mut self) {
        if self.items.is_empty() {
            return;
        }

        let depth = self.get_selected_item().depth;

        let last = BOTTOM_CONTEXT_LINES
            + (self.cursor..self.line_index.len())
                .take_while(|&line_i| line_i == self.cursor || depth < self.at_line(line_i).depth)
                .last()
                .unwrap();

        let end_line = self.size.height.saturating_sub(1) as usize;
        if last > end_line + self.scroll {
            self.scroll = last - end_line;
        }
    }

    pub(crate) fn find_next(&mut self, nav_mode: NavMode) -> usize {
        (self.cursor..self.line_index.len())
            .skip(1)
            .find(|&line_i| self.nav_filter(line_i, nav_mode))
            .unwrap_or(self.cursor)
    }

    fn nav_filter(&self, line_i: usize, nav_mode: NavMode) -> bool {
        let item = self.at_line(line_i);
        match nav_mode {
            NavMode::Normal => {
                let is_sub_line = matches!(
                    item.data,
                    ItemData::HunkLine { .. } | ItemData::BlameCodeLine { .. }
                );
                !item.unselectable && !is_sub_line
            }
            NavMode::Siblings { depth } => {
                !item.unselectable && item.data.is_section() && item.depth <= depth
            }
            NavMode::IncludeSubLines => !item.unselectable,
        }
    }

    pub(crate) fn select_previous(&mut self, nav_mode: NavMode) {
        self.anchor = None;
        self.move_previous(nav_mode);
    }

    fn move_previous(&mut self, nav_mode: NavMode) {
        self.cursor = self.find_previous(nav_mode);
        self.scroll_fit_start();
    }

    /// Grow (or shrink) the selection by one line, so that a run of lines can be
    /// staged, discarded or reversed as one patch. It stays inside a single hunk,
    /// which is as far as one patch reaches.
    pub(crate) fn extend_selection(&mut self, forwards: bool) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        let next = if forwards {
            self.find_next(NavMode::IncludeSubLines)
        } else {
            self.find_previous(NavMode::IncludeSubLines)
        };

        if !same_hunk(&self.at_line(anchor).data, &self.at_line(next).data) {
            return;
        }

        self.anchor = Some(anchor);
        self.cursor = next;
        self.scroll_fit_end();
        self.scroll_fit_start();
    }

    /// The lines the selection spans; just the cursor's line when none is being
    /// made.
    fn selection(&self) -> RangeInclusive<usize> {
        let anchor = self.anchor.unwrap_or(self.cursor);
        anchor.min(self.cursor)..=anchor.max(self.cursor)
    }

    /// What an op acts on. A multi-line selection reads as the line under the
    /// cursor widened to cover every line selected, so that ops staging a
    /// `HunkLine` take them all in a single patch without knowing about
    /// selections.
    pub(crate) fn selected_target(&self) -> ItemData {
        let ItemData::HunkLine {
            diff,
            file_i,
            hunk_i,
            line_i,
            line_range,
            ..
        } = &self.get_selected_item().data
        else {
            return self.get_selected_item().data.clone();
        };

        // Context lines within the selection contribute nothing: a patch leaves
        // them be whether or not they are named.
        let mut line_indices = self
            .selection()
            .filter_map(|line| match &self.at_line(line).data {
                ItemData::HunkLine { line_indices, .. } => Some(line_indices),
                _ => None,
            })
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        line_indices.sort_unstable();
        line_indices.dedup();

        ItemData::HunkLine {
            diff: Rc::clone(diff),
            file_i: *file_i,
            hunk_i: *hunk_i,
            line_i: *line_i,
            line_range: line_range.clone(),
            line_indices,
        }
    }

    fn find_previous(&mut self, nav_mode: NavMode) -> usize {
        (0..self.cursor)
            .rev()
            .find(|&line_i| self.nav_filter(line_i, nav_mode))
            .unwrap_or(self.cursor)
    }

    pub(crate) fn scroll_view_half_page_up(&mut self) {
        let half_screen = self.size.height as usize / 2;
        self.scroll_view_up(half_screen);
    }

    pub(crate) fn scroll_view_half_page_down(&mut self) {
        let half_screen = self.size.height as usize / 2;
        self.scroll_view_down(half_screen);
    }

    pub(crate) fn scroll_view_up(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
        self.clamp_scroll();
    }

    pub(crate) fn scroll_view_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_add(lines);
        self.clamp_scroll();
    }

    pub(crate) fn toggle_section(&mut self) {
        self.anchor = None;
        let selected = &self.items[self.line_index[self.cursor]];

        if selected.data.is_section() {
            if self.collapsed.contains(&selected.id) {
                self.collapsed.remove(&selected.id);
            } else {
                self.collapsed.insert(selected.id);
            }
        }

        self.update_line_index();
    }

    pub(crate) fn update(&mut self) -> Res<()> {
        let nav_mode = self.selected_item_nav_mode();
        // The rebuilt items are a different diff; the lines that were selected
        // are no longer the same lines.
        self.anchor = None;
        self.items = (self.refresh_items)(self.size)?;
        self.update_line_index();
        self.update_cursor(nav_mode);
        Ok(())
    }

    fn update_cursor(&mut self, nav_mode: NavMode) {
        // Nothing is selectable (e.g. the log of a branch with no commits).
        // Reset the cursor to a valid sentinel rather than positioning it,
        // which would index into an empty `line_index` and panic (#262).
        if self.line_index.is_empty() {
            self.cursor = 0;
            return;
        }

        self.clamp_scroll();
        self.clamp_cursor();
        if self.is_cursor_off_screen() {
            self.move_cursor_to_screen_center();
        }

        self.clamp_cursor();
        self.move_from_unselectable(nav_mode);
    }

    fn selected_item_nav_mode(&mut self) -> NavMode {
        if self.items.is_empty() {
            return NavMode::Normal;
        }

        match self.get_selected_item().data {
            ItemData::HunkLine { .. } | ItemData::BlameCodeLine { .. } => NavMode::IncludeSubLines,
            _ => NavMode::Normal,
        }
    }

    fn update_line_index(&mut self) {
        self.line_index = self
            .items
            .iter()
            .enumerate()
            .scan(None, |collapse_depth, (i, next)| {
                if collapse_depth.is_some_and(|depth| depth < next.depth) {
                    return Some(None);
                }

                *collapse_depth = if next.data.is_section() && self.is_collapsed(next) {
                    Some(next.depth)
                } else {
                    None
                };

                Some(Some((i, next)))
            })
            .flatten()
            .map(|(i, _item)| i)
            .collect();
        self.clamp_scroll();
    }

    fn is_cursor_off_screen(&self) -> bool {
        !self.line_views(self.size).any(|line| line.highlighted)
    }

    fn move_cursor_to_screen_center(&mut self) {
        let half_screen = self.size.height as usize / 2;
        self.cursor = self.scroll + half_screen;
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self
            .cursor
            .clamp(0, self.line_index.len().saturating_sub(1));
    }

    fn clamp_scroll(&mut self) {
        if self.line_index.is_empty() {
            self.scroll = 0;
            return;
        }

        self.scroll = self.scroll.min(self.max_scroll_with_context());
    }

    fn max_scroll_with_context(&self) -> usize {
        let len = self.line_index.len();
        if len == 0 {
            return 0;
        }

        let max_scroll = len.saturating_sub(self.size.height as usize);
        let max_scroll = max_scroll.saturating_add(BOTTOM_CONTEXT_LINES);
        max_scroll.min(len.saturating_sub(1))
    }

    fn move_from_unselectable(&mut self, nav_mode: NavMode) {
        if !self.nav_filter(self.cursor, nav_mode) {
            self.move_previous(nav_mode);
        }
        if !self.nav_filter(self.cursor, nav_mode) {
            self.move_next(nav_mode);
        }
    }

    pub(crate) fn move_cursor_to_screen_line(&mut self, screen_line: usize) {
        if self.line_index.is_empty() {
            return;
        }

        let new_cursor = screen_line + self.scroll;
        if new_cursor >= self.line_index.len() || self.cursor == new_cursor {
            return;
        }

        let old_cursor = self.cursor;
        self.anchor = None;
        self.cursor = new_cursor;

        let nav_mode = self.selected_item_nav_mode();
        self.move_from_unselectable(nav_mode);

        if !self.nav_filter(self.cursor, nav_mode) {
            // There was no selectable item, put the cursor back.
            self.cursor = old_cursor;
        } else {
            // Use minimal scrolling to keep the cursor visible.
            self.scroll_fit_start();
        }
    }

    pub(crate) fn move_cursor_to_top(&mut self) {
        if self.line_index.is_empty() {
            return;
        }
        if let Some(first) = self.find_first_selectable() {
            self.anchor = None;
            self.cursor = first;
            self.scroll = 0;
        }
    }

    pub(crate) fn move_cursor_to_bottom(&mut self) {
        if self.line_index.is_empty() {
            return;
        }
        if let Some(last) = self.find_last_selectable() {
            self.anchor = None;
            self.cursor = last;
            self.scroll_fit_end();
        }
    }

    fn find_last_selectable(&self) -> Option<usize> {
        (0..self.line_index.len()).rfind(|&line_i| !self.at_line(line_i).unselectable)
    }

    pub(crate) fn is_collapsed(&self, item: &Item) -> bool {
        self.collapsed.contains(&item.id)
    }

    pub(crate) fn get_selected_item(&self) -> &Item {
        &self.items[self.line_index[self.cursor]]
    }

    pub(crate) fn select_matching<F: Fn(&ItemData) -> bool>(&mut self, predicate: F) -> bool {
        let Some(line_i) = self.find_matching(predicate) else {
            return false;
        };

        self.cursor = line_i;
        let half_screen = self.size.height as usize / 2;
        if self.cursor >= half_screen {
            self.scroll = self.cursor - half_screen;
        }
        self.scroll_fit_end();
        self.scroll_fit_start();
        true
    }

    /// As [`Self::select_matching`], but the rows stay where they are on screen:
    /// the view scrolls only as far as it takes to bring the cursor into it.
    pub(crate) fn select_matching_in_view<F: Fn(&ItemData) -> bool>(
        &mut self,
        predicate: F,
    ) -> bool {
        let Some(line_i) = self.find_matching(predicate) else {
            return false;
        };

        self.cursor = line_i;
        self.scroll_fit_end();
        self.scroll_fit_start();
        true
    }

    fn find_matching<F: Fn(&ItemData) -> bool>(&self, predicate: F) -> Option<usize> {
        (0..self.line_index.len()).find(|&line_i| {
            !self.at_line(line_i).unselectable && predicate(&self.at_line(line_i).data)
        })
    }

    pub(crate) fn select_last_matching<F: Fn(&ItemData) -> bool>(&mut self, predicate: F) -> bool {
        if let Some(line_i) = (0..self.line_index.len()).rev().find(|&line_i| {
            !self.at_line(line_i).unselectable && predicate(&self.at_line(line_i).data)
        }) {
            self.cursor = line_i;
            let half_screen = self.size.height as usize / 2;
            if self.cursor >= half_screen {
                self.scroll = self.cursor - half_screen;
            } else {
                self.scroll_fit_start();
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn is_valid_screen_line(&self, screen_line: usize) -> bool {
        let target_line_i = screen_line + self.scroll;
        if self.line_index.is_empty() || target_line_i >= self.line_index.len() {
            return false;
        }
        self.nav_filter(target_line_i, NavMode::IncludeSubLines)
    }

    fn line_views(&'_ self, area: Size) -> impl Iterator<Item = LineView<'_>> {
        let selection = self.selection();
        let scan_start = self.scroll.min(*selection.start());
        let scan_end = (self.scroll + area.height as usize).min(self.line_index.len());
        let context_lines = self.scroll - scan_start;

        (scan_start..scan_end)
            .scan(None, move |highlight_depth, line_i| {
                let item_index = self.line_index[line_i];
                let item = &self.items[item_index];
                let selected = selection.contains(&line_i);
                if selected {
                    *highlight_depth = Some(item.depth);
                } else if highlight_depth.is_some_and(|s| s >= item.depth) {
                    *highlight_depth = None;
                };

                Some(LineView {
                    item_index,
                    display: item.to_line(Arc::clone(&self.config)),
                    highlighted: highlight_depth.is_some(),
                    selected,
                })
            })
            .skip(context_lines)
    }
}

/// Whether two rows are content lines of one hunk, and so can be selected
/// together: a patch reaches no further than that.
fn same_hunk(a: &ItemData, b: &ItemData) -> bool {
    match (a, b) {
        (
            ItemData::HunkLine {
                diff,
                file_i,
                hunk_i,
                ..
            },
            ItemData::HunkLine {
                diff: b_diff,
                file_i: b_file_i,
                hunk_i: b_hunk_i,
                ..
            },
        ) => Rc::ptr_eq(diff, b_diff) && file_i == b_file_i && hunk_i == b_hunk_i,
        _ => false,
    }
}

struct LineView<'a> {
    item_index: usize,
    display: Line<'a>,
    highlighted: bool,
    /// Drawn as the cursor line is: the selection may span several of these.
    selected: bool,
}

const SPACES: &str = "                                                                ";

pub(crate) fn layout_screen<'a>(
    layout: &mut UiTree<'a>,
    size: Size,
    screen: &'a Screen,
    hide_cursor: bool,
) {
    let style = &screen.config.style;

    layout.vertical(None, OPTS, |layout| {
        for line in screen.line_views(size) {
            layout.horizontal(None, OPTS, |layout| {
                let is_line_sel = line.selected;
                let area_sel = area_selection_highlight(style, &line);
                let line_sel = line_selection_highlight(style, &line, is_line_sel);
                let bg = area_sel.patch(line_sel);

                let mut line_end = 1;
                let gutter_char = if !hide_cursor && line.highlighted {
                    gutter_char(style, is_line_sel, bg)
                } else {
                    (" ".into(), Style::new())
                };

                layout_span(layout, gutter_char);

                line.display.spans.into_iter().for_each(|span| {
                    let style = bg.patch(line.display.style).patch(span.style);

                    let span_width = span.content.graphemes(true).count();

                    if line_end + span_width >= size.width as usize {
                        // Truncate the span and insert an ellipsis to indicate overflow
                        let overflow = line_end + span_width - size.width as usize;
                        line_end = size.width as usize;
                        ui::layout_span(
                            layout,
                            (
                                span.content
                                    .graphemes(true)
                                    .take(span_width.saturating_sub(overflow + 1))
                                    .collect::<String>()
                                    .into(),
                                style,
                            ),
                        );
                        layout_span(layout, ("…".into(), bg));
                    } else {
                        // Insert the span as normal
                        line_end += span_width;
                        ui::layout_span(layout, (span.content, style));
                    }
                });

                // Add ellipsis indicator for collapsed sections that hide children
                let item = &screen.items[line.item_index];
                let has_children = screen
                    .items
                    .get(line.item_index + 1)
                    .is_some_and(|next| next.depth > item.depth);
                if has_children && screen.is_collapsed(item) {
                    line_end += 1;
                    layout_span(layout, ("…".into(), bg));
                }

                // Style the rest of the line's empty space
                let style = if is_line_sel { line_sel } else { area_sel };
                let padding_width = (size.width as usize).saturating_sub(line_end);
                ui::repeat_chars(layout, padding_width, SPACES, style);
            });
        }
    });
}

fn gutter_char<'a>(style: &'a StyleConfig, is_line_sel: bool, bg: Style) -> (Cow<'a, str>, Style) {
    if is_line_sel {
        (
            style.cursor.symbol.to_string().into(),
            bg.patch(Style::from(&style.cursor)),
        )
    } else {
        (
            style.selection_bar.symbol.to_string().into(),
            bg.patch(Style::from(&style.selection_bar)),
        )
    }
}

fn line_selection_highlight(style: &StyleConfig, line: &LineView, selected_line: bool) -> Style {
    if line.highlighted && selected_line {
        Style::from(&style.selection_line)
    } else {
        Style::new()
    }
}

fn area_selection_highlight(style: &StyleConfig, line: &LineView) -> Style {
    if line.highlighted {
        Style::from(&style.selection_area)
    } else {
        Style::new()
    }
}
