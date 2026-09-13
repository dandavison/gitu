use crate::config::StyleConfig;
use crate::error::Error;
use crate::search::RunText;
use crate::style::Style;
use crate::ui::layout::{LayoutTree, opts};
use crate::ui::{UiTree, layout_span};
use crate::{
    Res,
    config::Config,
    items::{ItemId, RenderParams, hash},
};
use crate::{item_data::ItemData, ui};

use super::Item;
use regex::{Regex, RegexBuilder};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::iter::successors;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

pub(crate) mod blame;
pub(crate) mod log;
pub(crate) mod pager;
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

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Scroll {
    item_anchor: usize,
    offset: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}

impl SearchDirection {
    fn reverse(self) -> Self {
        match self {
            SearchDirection::Forward => SearchDirection::Backward,
            SearchDirection::Backward => SearchDirection::Forward,
        }
    }
}

/// The search to repeat, as `n` and `N` do in vim.
struct Search {
    query: String,
    matcher: Regex,
    direction: SearchDirection,
}

/// Compiles `query` as a regex, ignoring case unless the query has uppercase in
/// it.
pub(crate) fn matcher(query: &str) -> Res<Regex> {
    RegexBuilder::new(query)
        .case_insensitive(!has_uppercase(query))
        .build()
        .map_err(Error::InvalidSearchRegex)
}

/// Whether the query asks for a case of its own. Skips e.g. `\W`, `\S`.
fn has_uppercase(query: &str) -> bool {
    let mut chars = query.chars();

    while let Some(char) = chars.next() {
        match char {
            '\\' => _ = chars.next(),
            _ if char.is_uppercase() => return true,
            _ => (),
        }
    }

    false
}

/// Builds a screen's items, given what to render them with: the viewport to
/// fill, and the renderer features and context chosen in-session.
pub(crate) type RefreshItems = Box<dyn Fn(RenderParams) -> Res<Vec<Item>>>;

pub(crate) struct Screen {
    pub(crate) size: (u16, u16),
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
    scroll: Scroll,
    config: Arc<Config>,
    /// The renderer features to rebuild with, as chosen in-session. Pushed down
    /// from the app so a screen always renders with the current selection.
    pub(crate) features: Rc<[String]>,
    /// How much context to ask git for, pushed down from the app as the
    /// features are.
    pub(crate) context: Option<Rc<str>>,
    /// The command this screen is showing the output of, where it is one gitu
    /// can put again. Shared with the closure that rebuilds the screen, so that
    /// editing it is what the next rebuild asks.
    pub(crate) git_command: Option<Rc<RefCell<crate::calling_process::GitCommand>>>,
    refresh_items: RefreshItems,
    items: Vec<Item>,
    /// Memoized `item_height`, indexed like `items`. Dropped by `invalidate`.
    item_heights: RefCell<Vec<Option<u16>>>,
    collapsed: HashSet<u64>,
    search: Option<Search>,
}

impl Screen {
    pub(crate) fn new(
        config: Arc<Config>,
        params: RenderParams,
        refresh_items: RefreshItems,
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
            scroll: Scroll::default(),
            size: params.size,
            config,
            features: params.features,
            context: params.context,
            git_command: None,
            refresh_items,
            items: vec![],
            item_heights: RefCell::new(vec![]),
            collapsed,
            search: None,
        };

        screen.items = fill_empty((screen.refresh_items)(screen.params())?);

        // TODO Maybe this should be done on update. Better keep track of toggled sections rather than collapsed then.
        screen.collapsed.extend(
            screen
                .items
                .iter()
                .filter(|item| item.default_collapsed)
                .map(|item| item.id),
        );

        screen.invalidate();
        screen.update_cursor();

        screen.cursor = screen
            .find_first_hunk()
            .or_else(|| screen.find_item(|item| !item.unselectable))
            .unwrap_or(0);

        Ok(screen)
    }

    fn params(&self) -> RenderParams {
        RenderParams {
            size: self.size,
            features: Rc::clone(&self.features),
            context: self.context.clone(),
        }
    }

    fn find_first_hunk(&mut self) -> Option<usize> {
        self.find_item(|item| !item.unselectable && matches!(item.data, ItemData::Hunk { .. }))
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

    /// Scrolls up until the cursor is on screen, keeping as many lines above it
    /// as it is deep, so that the headers it sits under stay in view.
    fn scroll_fit_start(&mut self) {
        if self.items.is_empty() {
            return;
        }

        let at_cursor = Scroll {
            item_anchor: self.cursor,
            offset: 0,
        };

        let top = self.retreated(at_cursor, self.items[self.cursor].depth);
        self.scroll = self.scroll.min(top);
    }

    /// Scrolls down until the selection ends on screen, keeping
    /// `BOTTOM_CONTEXT_LINES` below it.
    fn scroll_fit_end(&mut self) {
        if self.items.is_empty() {
            return;
        }

        let last = self.last_selected();
        let at_last_line = Scroll {
            item_anchor: last,
            offset: self.item_height(last).saturating_sub(1),
        };

        let bottom = self.retreated(at_last_line, self.rows_above_context());
        self.scroll = self.scroll.max(bottom);
    }

    /// The last row the content may occupy before running into the bottom
    /// context lines.
    fn rows_above_context(&self) -> usize {
        (self.size.1 as usize).saturating_sub(BOTTOM_CONTEXT_LINES + 1)
    }

    /// The last item of the selection: the cursor, or the end of the subtree
    /// it heads.
    fn last_selected(&self) -> usize {
        let depth = self.items[self.cursor].depth;

        let last = self.items[self.cursor + 1..]
            .iter()
            .position(|item| item.depth <= depth)
            .map_or(self.items.len() - 1, |offset| self.cursor + offset);

        // A collapsed section within the selection stands in for its contents.
        self.hidden_by(last, depth).unwrap_or(last)
    }

    pub(crate) fn find_next(&mut self, nav_mode: NavMode) -> usize {
        self.visible_from(self.cursor)
            .skip(1)
            .find(|&item_i| self.nav_filter(item_i, nav_mode))
            .unwrap_or(self.cursor)
    }

    /// Whether `item_i` may be selected, disregarding whether it is hidden by
    /// a collapsed ancestor.
    fn nav_filter(&self, item_i: usize, nav_mode: NavMode) -> bool {
        let item = &self.items[item_i];
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

    fn is_selectable(&self, item_i: usize, nav_mode: NavMode) -> bool {
        self.is_visible(item_i) && self.nav_filter(item_i, nav_mode)
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

        if !same_hunk(&self.items[anchor].data, &self.items[next].data) {
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
            .filter_map(|line| match &self.items[line].data {
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
        self.visible_before(self.cursor)
            .find(|&item_i| self.nav_filter(item_i, nav_mode))
            .unwrap_or(self.cursor)
    }

    pub(crate) fn half_page_up(&mut self) {
        self.turn_page(self.size.1 as usize / 2, false);
    }

    pub(crate) fn half_page_down(&mut self) {
        self.turn_page(self.size.1 as usize / 2, true);
    }

    /// A page is a whole viewport, which is two half pages.
    pub(crate) fn full_page_up(&mut self) {
        self.turn_page(self.size.1 as usize, false);
    }

    pub(crate) fn full_page_down(&mut self) {
        self.turn_page(self.size.1 as usize, true);
    }

    /// Move the view by `lines`, taking the cursor with it. Once the view has
    /// nowhere further to go the cursor finishes the journey on its own, so a
    /// run of rows with nothing to select on it — a commit message taller than
    /// the screen — can still be turned past.
    fn turn_page(&mut self, lines: usize, forwards: bool) {
        self.anchor = None;
        let row = self.cursor_row();
        let before = self.scroll;

        if forwards {
            self.scroll_view_down(lines);
        } else {
            self.scroll_view_up(lines);
        }

        if self.scroll == before {
            if forwards {
                self.move_cursor_to_bottom();
            } else {
                self.move_cursor_to_top();
            }
            return;
        }

        // The cursor keeps its row, so turning the page back arrives where it
        // started. Fitting it would scroll back to the page it came from, so
        // the view is put back where the page left it.
        let scroll = self.scroll;
        if let Some(row) = row {
            self.move_cursor_to_screen_line(row);
        }
        let nav_mode = self.selected_item_nav_mode();
        self.move_from_unselectable(nav_mode);
        self.scroll = scroll;
    }

    /// Which row of the viewport the cursor is drawn on, when it is on screen.
    fn cursor_row(&self) -> Option<usize> {
        self.screen_rows()
            .find(|&(item_i, _)| item_i == self.cursor)
            .map(|(_, row)| row)
    }

    pub(crate) fn scroll_view_up(&mut self, lines: usize) {
        self.retreat(lines);
    }

    pub(crate) fn scroll_view_down(&mut self, lines: usize) {
        self.advance(lines);
        self.clamp_scroll();
    }

    /// Fold the view down to its outermost headings, or open all of it when
    /// they are already folded. Only the outermost are folded, and everything
    /// within them is opened, so that opening one shows what is inside it
    /// rather than another folded thing. The cursor can end up inside what was
    /// folded away, so it is re-placed afterwards.
    pub(crate) fn toggle_all_sections(&mut self) {
        self.anchor = None;

        let sections = || self.items.iter().filter(|item| item.data.is_section());
        let Some(outermost) = sections().map(|item| item.depth).min() else {
            return;
        };
        let headings: Vec<ItemId> = sections()
            .filter(|item| item.depth == outermost)
            .map(|item| item.id)
            .collect();

        let folded = headings.iter().all(|id| self.collapsed.contains(id));
        self.collapsed.clear();
        if !folded {
            self.collapsed.extend(headings);
        }

        self.invalidate();
        self.update_cursor();
    }

    pub(crate) fn toggle_section(&mut self) -> Res<()> {
        self.anchor = None;
        let selected = &self.items[self.cursor];

        if selected.data.is_section() {
            let id = selected.id;
            if !self.collapsed.remove(&id) {
                self.collapsed.insert(id);
            }

            // Only the toggled section's own height changes, by gaining or
            // losing its `…`. Everything else lays out from its own content at
            // an unchanged width, so those memoized heights still hold.
            self.item_heights.get_mut()[self.cursor] = None;

            // Collapsing the section the viewport sits in takes it along.
            self.fit_anchor();
        }

        self.clamp_scroll();
        Ok(())
    }

    pub(crate) fn refresh(&mut self) -> Res<()> {
        // The rebuilt items are a different diff; the lines that were selected
        // are no longer the same lines.
        self.anchor = None;
        self.items = fill_empty((self.refresh_items)(self.params())?);

        self.invalidate();
        self.update_cursor();
        Ok(())
    }

    pub(crate) fn resize(&mut self, w: u16, h: u16) -> Res<()> {
        self.fit_to(w, h);
        self.update_cursor();
        Ok(())
    }

    /// Take the rows the screen has been given, which is what the panels below
    /// it leave. Measured item heights hold at an unchanged size, and every
    /// frame asks for this one. The cursor is left where it is: this is not the
    /// user moving it.
    pub(crate) fn fit_to(&mut self, w: u16, h: u16) {
        if self.size == (w, h) {
            return;
        }

        self.size = (w, h);
        self.invalidate();
    }

    /// Rebuild, keeping the cursor on whatever it was on. [`Self::refresh`]
    /// keeps its place in the list, which is the same place only while the rows
    /// are; a re-render that lays the same diff out differently moves them.
    /// What does not move is where a row sits in the patch, so that is what is
    /// restored.
    pub(crate) fn refresh_keeping_position(&mut self) -> Res<()> {
        let was_on = diff_position(&self.get_selected_item().data);
        self.refresh()?;

        if let Some(position) = was_on {
            self.select_matching_in_view(|data| diff_position(data) == Some(position));
        }
        Ok(())
    }

    fn update_cursor(&mut self) {
        // Nothing is selectable (e.g. the log of a branch with no commits).
        // Reset the cursor to a valid sentinel rather than positioning it,
        // which would index into a screen with no lines and panic (#262).
        if self.is_empty() {
            self.cursor = 0;
            return;
        }

        self.clamp_scroll();
        self.clamp_cursor();
        if self.is_cursor_off_screen() {
            self.move_cursor_to_screen_center();
        }

        self.clamp_cursor();
        let nav_mode = self.selected_item_nav_mode();
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

    /// Drops everything derived from `items` and `size`. The accessors below
    /// recompute what they need, when they need it.
    fn invalidate(&mut self) {
        let item_heights = self.item_heights.get_mut();
        item_heights.clear();
        item_heights.resize(self.items.len(), None);

        self.fit_anchor();
        self.clamp_scroll();
    }

    /// Puts the viewport back on an item that exists and is visible, after the
    /// items changed underneath it.
    fn fit_anchor(&mut self) {
        if self.items.is_empty() {
            self.scroll = Scroll::default();
            return;
        }

        self.scroll.item_anchor = self.scroll.item_anchor.min(self.items.len() - 1);

        if let Some(section) = self.hidden_by(self.scroll.item_anchor, 0) {
            self.scroll = Scroll {
                item_anchor: section,
                offset: 0,
            };
        }
    }

    /// The visible item after `item_i`, which only `item_i` itself can hide.
    fn next_visible(&self, item_i: usize) -> Option<usize> {
        let item = &self.items[item_i];
        let next = item_i + 1;

        if !item.data.is_section() || !self.is_collapsed(item) {
            return (next < self.items.len()).then_some(next);
        }

        // Step over the subtree it holds shut.
        let shut = self.items[next..]
            .iter()
            .position(|below| below.depth <= item.depth)?;

        Some(next + shut)
    }

    /// The visible item before `item_i`.
    fn prev_visible(&self, item_i: usize) -> Option<usize> {
        let prev = item_i.checked_sub(1)?;

        // In case prev is collapsed, we need to find the item that hides it.
        let depth = self.items[item_i].depth;
        Some(self.hidden_by(prev, depth).unwrap_or(prev))
    }

    /// The outermost collapsed section hiding `item_i`, looking no further out
    /// than `floor`.
    fn hidden_by(&self, item_i: usize, floor: usize) -> Option<usize> {
        let mut depth = self.items[item_i].depth;
        if depth <= floor {
            return None;
        }

        let mut hidden_by = None;

        for (i, ancestor) in self.items[..item_i].iter().enumerate().rev() {
            if ancestor.depth >= depth {
                continue;
            }

            depth = ancestor.depth;
            if ancestor.data.is_section() && self.is_collapsed(ancestor) {
                hidden_by = Some(i);
            }

            if depth <= floor {
                break;
            }
        }

        hidden_by
    }

    /// Visible items at or after `item_i`, in order.
    fn visible_from(&self, item_i: usize) -> impl Iterator<Item = usize> {
        let start = (item_i < self.items.len()).then_some(item_i);
        successors(start, |&item_i| self.next_visible(item_i))
    }

    /// Visible items before `item_i`, in reverse order.
    fn visible_before(&self, item_i: usize) -> impl Iterator<Item = usize> {
        successors(self.prev_visible(item_i), |&item_i| {
            self.prev_visible(item_i)
        })
    }

    /// Every visible item, from the top of the content down.
    fn visible_items(&self) -> impl Iterator<Item = usize> {
        self.visible_from(0)
    }

    /// Every visible item, from the bottom of the content up.
    fn visible_items_rev(&self) -> impl Iterator<Item = usize> {
        let last = self
            .items
            .len()
            .checked_sub(1)
            .map(|last| self.hidden_by(last, 0).unwrap_or(last));

        successors(last, |&item_i| self.prev_visible(item_i))
    }

    fn is_visible(&self, item_i: usize) -> bool {
        self.hidden_by(item_i, 0).is_none()
    }

    /// How many lines `item_i` occupies once laid out at the current width.
    fn item_height(&self, item_i: usize) -> usize {
        if let Some(height) = self.item_heights.borrow()[item_i] {
            return height as usize;
        }

        // TODO A new allocation per measured item seems wasteful
        let mut layout = LayoutTree::new();
        let view = ItemView {
            item_index: item_i,
            highlighted: false,
            selected: false,
        };
        layout_item(&mut layout, self, false, view);

        let height = layout
            .compute([self.size.0, self.size.1])
            .iter()
            .map(|item| item.pos[1] + item.size[1])
            .max()
            .unwrap_or(0);

        self.item_heights.borrow_mut()[item_i] = Some(height);
        height as usize
    }

    /// The visible items from the top of the viewport down.
    fn items_from_anchor(&self) -> impl Iterator<Item = usize> {
        self.visible_from(self.scroll.item_anchor)
    }

    fn screen_rows(&self) -> impl Iterator<Item = (usize, usize)> {
        self.items_from_anchor()
            .scan(0, move |row, item_i| {
                let at = *row;
                *row += self.item_height(item_i);
                Some((item_i, at))
            })
            .take_while(move |&(item_i, at)| at + self.item_height(item_i) <= self.size.1 as usize)
    }

    /// The item drawn at `row` of the viewport, if the content reaches it.
    fn item_at_row(&self, row: usize) -> Option<usize> {
        self.screen_rows()
            .take_while(|&(_, at)| at <= row)
            .last()
            .filter(|&(item_i, at)| row < at + self.item_height(item_i))
            .map(|(item_i, _)| item_i)
    }

    /// Moves the top of the viewport `lines` further down the content, coming
    /// to rest on the last line there is.
    fn advance(&mut self, mut lines: usize) {
        if self.items.is_empty() {
            return;
        }

        while lines > 0 {
            let height = self.item_height(self.scroll.item_anchor);
            let left = height.saturating_sub(self.scroll.offset);

            if lines < left {
                self.scroll.offset += lines;
                return;
            }

            let Some(next) = self.next_visible(self.scroll.item_anchor) else {
                self.scroll.offset = height.saturating_sub(1);
                return;
            };

            lines -= left;
            self.scroll = Scroll {
                item_anchor: next,
                offset: 0,
            };
        }
    }

    /// Moves the top of the viewport `lines` back up the content, coming to
    /// rest at the top of it.
    fn retreat(&mut self, lines: usize) {
        self.scroll = self.retreated(self.scroll, lines);
    }

    /// `scroll` taken `lines` back up the content, coming to rest at the top
    /// of it.
    fn retreated(&self, mut scroll: Scroll, mut lines: usize) -> Scroll {
        if lines <= scroll.offset {
            scroll.offset -= lines;
            return scroll;
        }

        lines -= scroll.offset;
        scroll.offset = 0;

        while lines > 0 {
            let Some(prev) = self.prev_visible(scroll.item_anchor) else {
                break;
            };

            let height = self.item_height(prev);
            scroll = Scroll {
                item_anchor: prev,
                offset: height.saturating_sub(lines),
            };

            if lines <= height {
                break;
            }

            lines -= height;
        }

        scroll
    }

    /// Lines of content from the top of the viewport down, counting no further
    /// than `limit` so that nothing below the viewport is laid out.
    fn lines_below_anchor(&self, limit: usize) -> usize {
        if self.items.is_empty() {
            return 0;
        }

        let mut lines = self
            .item_height(self.scroll.item_anchor)
            .saturating_sub(self.scroll.offset);

        for item_i in self.items_from_anchor().skip(1) {
            if lines >= limit {
                break;
            }

            lines += self.item_height(item_i);
        }

        lines
    }

    /// Lines of content above the top of the viewport, counting no further
    /// than `limit`.
    fn lines_above_anchor(&self, limit: usize) -> usize {
        let mut lines = self.scroll.offset;
        let mut item_i = self.scroll.item_anchor;

        while lines < limit {
            let Some(prev) = self.prev_visible(item_i) else {
                break;
            };

            lines += self.item_height(prev);
            item_i = prev;
        }

        lines.min(limit)
    }

    /// Whether the screen renders no lines at all, e.g. the log of a branch
    /// with no commits.
    fn is_empty(&self) -> bool {
        !self
            .visible_items()
            .any(|item_i| self.item_height(item_i) > 0)
    }

    fn is_cursor_off_screen(&self) -> bool {
        !self.item_views().any(|item| item.highlighted)
    }

    fn move_cursor_to_screen_center(&mut self) {
        let half_screen = self.size.1 as usize / 2;

        // Scrolling is allowed to run past the content, so the middle of the
        // screen may hold no item at all. Fall back to the last one drawn.
        let center = self
            .item_at_row(half_screen)
            .or_else(|| self.screen_rows().last().map(|(item_i, _)| item_i));

        if let Some(item_i) = center {
            self.cursor = item_i;
        }
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.clamp(0, self.items.len().saturating_sub(1));
    }

    /// Pulls the viewport back up if it has scrolled further than the content
    /// allows.
    fn clamp_scroll(&mut self) {
        let wanted = (self.size.1 as usize).saturating_sub(BOTTOM_CONTEXT_LINES);
        let below = self.lines_below_anchor(wanted);
        if below >= wanted {
            return;
        }

        let over = wanted - below;
        let above = self.lines_above_anchor(over + BOTTOM_CONTEXT_LINES);
        self.retreat(over.min(above.saturating_sub(BOTTOM_CONTEXT_LINES)));
    }

    fn move_from_unselectable(&mut self, nav_mode: NavMode) {
        if !self.is_selectable(self.cursor, nav_mode) {
            self.move_previous(nav_mode);
        }
        if !self.is_selectable(self.cursor, nav_mode) {
            self.move_next(nav_mode);
        }
    }

    pub(crate) fn move_cursor_to_screen_line(&mut self, screen_line: usize) {
        let Some(new_cursor) = self.item_at_row(screen_line) else {
            return;
        };
        if self.cursor == new_cursor {
            return;
        }

        let old_cursor = self.cursor;
        self.anchor = None;
        self.cursor = new_cursor;

        let nav_mode = self.selected_item_nav_mode();
        self.move_from_unselectable(nav_mode);

        if !self.is_selectable(self.cursor, nav_mode) {
            // There was no selectable item, put the cursor back.
            self.cursor = old_cursor;
        } else {
            // Use minimal scrolling to keep the cursor visible.
            self.scroll_fit_start();
        }
    }

    pub(crate) fn move_cursor_to_top(&mut self) {
        self.anchor = None;
        if let Some(first) = self.find_item(|item| !item.unselectable) {
            self.cursor = first;
            self.scroll = Scroll::default();
        }
    }

    pub(crate) fn move_cursor_to_bottom(&mut self) {
        self.anchor = None;
        if let Some(last) = self.rfind_item(|item| !item.unselectable) {
            self.cursor = last;
            self.scroll_fit_end();
        }
    }

    pub(crate) fn is_collapsed(&self, item: &Item) -> bool {
        self.collapsed.contains(&item.id)
    }

    /// The text of each row, for asserting on what a screen shows.
    #[cfg(test)]
    pub(crate) fn row_texts(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|item| item.rendered.as_ref())
            .map(|row| row.iter().map(|(text, _)| text.as_str()).collect())
            .collect()
    }

    pub(crate) fn get_selected_item(&self) -> &Item {
        &self.items[self.cursor]
    }

    pub(crate) fn select_matching<F: Fn(&ItemData) -> bool>(&mut self, predicate: F) -> bool {
        let Some(item_i) = self.find_item(|item| !item.unselectable && predicate(&item.data))
        else {
            return false;
        };

        self.cursor = item_i;
        self.center_on_cursor();
        self.scroll_fit_end();
        self.scroll_fit_start();

        true
    }

    pub(crate) fn select_last_matching<F: Fn(&ItemData) -> bool>(&mut self, predicate: F) -> bool {
        let Some(item_i) = self.rfind_item(|item| !item.unselectable && predicate(&item.data))
        else {
            return false;
        };

        self.cursor = item_i;
        self.center_on_cursor();
        self.scroll_fit_start();

        true
    }

    /// Draws the cursor halfway down the screen, or as near to it as the
    /// content above reaches.
    fn center_on_cursor(&mut self) {
        self.scroll = Scroll {
            item_anchor: self.cursor,
            offset: 0,
        };

        self.retreat(self.size.1 as usize / 2);
        self.clamp_scroll();
    }

    /// Moves the cursor to the nearest item whose text `query` matches, looking
    /// in `direction` and wrapping around the ends. `query` is a regex, matched
    /// ignoring case unless it has uppercase in it.
    pub(crate) fn search(&mut self, query: &str, direction: SearchDirection) -> Res<()> {
        debug_assert!(!query.is_empty());

        let matcher = matcher(query)?;
        let found = self.move_to_match(&matcher, direction);

        // Remembered even when nothing matched, so that `n` retries it.
        self.search = Some(Search {
            query: query.to_string(),
            matcher,
            direction,
        });

        if found {
            Ok(())
        } else {
            Err(Error::NoSearchMatch(query.to_string()))
        }
    }

    /// As [`Self::select_matching`], but the rows stay where they are on screen:
    /// the view scrolls only as far as it takes to bring the cursor into it.
    pub(crate) fn select_matching_in_view<F: Fn(&ItemData) -> bool>(
        &mut self,
        predicate: F,
    ) -> bool {
        let Some(item_i) = self.find_matching(predicate) else {
            return false;
        };

        self.cursor = item_i;
        self.scroll_fit_end();
        self.scroll_fit_start();
        true
    }

    fn find_matching<F: Fn(&ItemData) -> bool>(&self, predicate: F) -> Option<usize> {
        self.find_item(|item| !item.unselectable && predicate(&item.data))
    }

    /// Repeats the last search. `reverse` flips its direction, as `N` does in vim.
    pub(crate) fn search_repeat(&mut self, reverse: bool) -> Res<()> {
        let Some(search) = &self.search else {
            return Err(Error::NoPreviousSearch);
        };

        let query = search.query.clone();
        let matcher = search.matcher.clone();
        let direction = if reverse {
            search.direction.reverse()
        } else {
            search.direction
        };

        if self.move_to_match(&matcher, direction) {
            Ok(())
        } else {
            Err(Error::NoSearchMatch(query))
        }
    }

    /// Whether a match was found, in which case the cursor is on it.
    fn move_to_match(&mut self, matcher: &Regex, direction: SearchDirection) -> bool {
        let Some(item_i) = self.find_match(matcher, direction) else {
            return false;
        };

        self.reveal(item_i);
        self.cursor = item_i;
        self.scroll_to_cursor();
        true
    }

    fn reveal(&mut self, item_i: usize) {
        while let Some(section) = self.hidden_by(item_i, 0) {
            self.collapsed.remove(&self.items[section].id);

            // Only the opened section's own height changes, by losing its `…`.
            self.item_heights.get_mut()[section] = None;
        }

        self.clamp_scroll();
    }

    /// The nearest match from the cursor, wrapping around the ends.
    fn find_match(&self, matcher: &Regex, direction: SearchDirection) -> Option<usize> {
        match direction {
            SearchDirection::Forward => self
                .matching_items(matcher)
                .find(|&item_i| item_i > self.cursor)
                .or_else(|| self.matching_items(matcher).next()),
            SearchDirection::Backward => self
                .matching_items(matcher)
                .take_while(|&item_i| item_i < self.cursor)
                .last()
                .or_else(|| self.matching_items(matcher).last()),
        }
    }

    /// The items `matcher` finds something in.
    fn matching_items<'a>(&'a self, matcher: &'a Regex) -> impl Iterator<Item = usize> + 'a {
        (0..self.items.len()).filter(move |&item_i| self.item_matches(item_i, matcher))
    }

    fn item_matches(&self, item_i: usize, matcher: &Regex) -> bool {
        let mut layout = UiTree::new();
        let view = ItemView {
            item_index: item_i,
            highlighted: false,
            selected: false,
        };
        layout_item(&mut layout, self, false, view);

        let mut run_text = RunText::default();

        layout
            .leaf_runs()
            .any(|run| matcher.is_match(run_text.read(run.map(|(leaf, span)| (leaf, span.text())))))
    }

    pub(crate) fn clear_search(&mut self) {
        self.search = None;
    }

    pub(crate) fn get_search_matcher(&self) -> Option<&Regex> {
        self.search.as_ref().map(|search| &search.matcher)
    }

    fn scroll_to_cursor(&mut self) {
        if self.is_cursor_off_screen() {
            self.center_on_cursor();
        }

        self.scroll_fit_end();
        self.scroll_fit_start();
    }

    fn find_item<P: Fn(&Item) -> bool>(&self, predicate: P) -> Option<usize> {
        self.visible_items()
            .find(|&item_i| predicate(&self.items[item_i]))
    }

    fn rfind_item<P: Fn(&Item) -> bool>(&self, predicate: P) -> Option<usize> {
        self.visible_items_rev()
            .find(|&item_i| predicate(&self.items[item_i]))
    }

    pub(crate) fn is_valid_screen_line(&self, screen_line: usize) -> bool {
        let Some(target_item_i) = self.item_at_row(screen_line) else {
            return false;
        };
        self.nav_filter(target_item_i, NavMode::IncludeSubLines)
    }

    /// Whether `item_i` is within the selection or sits in the subtree its last
    /// line heads, and so draws highlighted.
    fn is_selected(&self, item_i: usize) -> bool {
        let selection = self.selection();
        let Some(depth) = self.items.get(*selection.start()).map(|item| item.depth) else {
            return false;
        };

        selection.contains(&item_i)
            || (item_i > *selection.end()
                && self.items[*selection.end() + 1..=item_i]
                    .iter()
                    .rev()
                    .all(|item| item.depth > depth))
    }

    fn item_views(&self) -> impl Iterator<Item = ItemView> {
        let selection = self.selection();
        let selection_depth = self.items.get(*selection.start()).map(|item| item.depth);

        // The selection may start above the viewport with the subtree it
        // highlights reaching into it, so the first item on screen has to say
        // for itself whether it is selected. The rest follow from it.
        self.screen_rows().scan(
            self.is_selected(self.scroll.item_anchor),
            move |highlighted, (item_index, _)| {
                if selection.contains(&item_index) {
                    *highlighted = true;
                } else if selection_depth.is_some_and(|depth| self.items[item_index].depth <= depth)
                {
                    *highlighted = false;
                }

                Some(ItemView {
                    item_index,
                    highlighted: *highlighted,
                    selected: selection.contains(&item_index),
                })
            },
        )
    }
}

/// A view is legitimately empty — nothing staged, the last hunk staged away, a
/// branch with no commits — and a screen still has to answer what is selected.
/// So it gets a row, which nothing can act on.
fn fill_empty(mut items: Vec<Item>) -> Vec<Item> {
    if items.is_empty() {
        items.push(Item {
            unselectable: true,
            ..Default::default()
        });
    }
    items
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

/// Where a row sits in the patch, for the rows that sit anywhere in one: a
/// file, a hunk within it, and for a content row the line within that. Two rows
/// of one wrapped line share a position, which is right — they are one line.
fn diff_position(data: &ItemData) -> Option<(usize, Option<usize>, Option<usize>)> {
    match data {
        ItemData::Delta { file_i, .. } => Some((*file_i, None, None)),
        ItemData::Hunk { file_i, hunk_i, .. } => Some((*file_i, Some(*hunk_i), None)),
        ItemData::HunkLine {
            file_i,
            hunk_i,
            line_i,
            ..
        } => Some((*file_i, Some(*hunk_i), Some(*line_i))),
        _ => None,
    }
}

struct ItemView {
    item_index: usize,
    highlighted: bool,
    /// Drawn as the cursor line is: the selection may span several of these.
    selected: bool,
}

pub(crate) fn layout_screen<'a>(layout: &mut UiTree<'a>, screen: &'a Screen, hide_cursor: bool) {
    layout.col(opts().fill_x(), |layout| {
        for view in screen.item_views() {
            layout_item(layout, screen, hide_cursor, view);
        }
    });
}

fn layout_item<'a>(layout: &mut UiTree<'a>, screen: &'a Screen, hide_cursor: bool, line: ItemView) {
    let style = &screen.config.style;
    let is_line_sel = line.selected;

    let area_sel = area_selection_highlight(style, &line);
    let line_sel = line_selection_highlight(style, &line, is_line_sel);
    let bg = area_sel.patch(line_sel);

    layout.row_with(bg, opts().fill_x(), |layout| {
        let gutter_char = if !hide_cursor && line.highlighted {
            gutter_char(style, is_line_sel, bg)
        } else {
            (" ".into(), Style::new())
        };

        // In a container of its own, so that it is not part of the row's text:
        // a search is over what the row says, not over its cursor bar.
        layout.row(opts(), |layout| layout_span(layout, gutter_char));

        let item = &screen.items[line.item_index];
        ui::item::layout_item(layout, item, &screen.config, bg);

        // A collapsed section says so only when it is hiding something.
        let has_children = screen
            .items
            .get(line.item_index + 1)
            .is_some_and(|next| next.depth > item.depth);
        if has_children && screen.is_collapsed(item) {
            layout_span(layout, ("…".into(), bg));
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

fn line_selection_highlight(style: &StyleConfig, line: &ItemView, selected_line: bool) -> Style {
    if line.highlighted && selected_line {
        Style::from(&style.selection_line)
    } else {
        Style::new()
    }
}

fn area_selection_highlight(style: &StyleConfig, line: &ItemView) -> Style {
    if line.highlighted {
        Style::from(&style.selection_area)
    } else {
        Style::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::init_test_config;
    use crate::item_data::SectionHeader;

    fn screen_of(item_count: usize, size: (u16, u16)) -> Screen {
        let config = Arc::new(init_test_config().unwrap());

        Screen::new(
            config,
            RenderParams {
                size,
                features: Rc::from([]),
                context: None,
            },
            Box::new(move |_params| {
                Ok((0..item_count)
                    .map(|i| Item {
                        id: i as u64,
                        data: ItemData::Raw(format!("item {i}")),
                        ..Default::default()
                    })
                    .collect())
            }),
        )
        .unwrap()
    }

    /// A screen of `(depth, is_section)` items, so that collapsing can be
    /// exercised without building a whole diff.
    fn tree_screen(spec: &[(usize, bool)], size: (u16, u16)) -> Screen {
        let config = Arc::new(init_test_config().unwrap());
        let spec = spec.to_vec();

        Screen::new(
            config,
            RenderParams {
                size,
                features: Rc::from([]),
                context: None,
            },
            Box::new(move |_params| {
                Ok(spec
                    .iter()
                    .enumerate()
                    .map(|(i, &(depth, is_section))| Item {
                        id: i as u64,
                        depth,
                        data: if is_section {
                            ItemData::Header(SectionHeader::Tags)
                        } else {
                            ItemData::Raw(format!("item {i}"))
                        },
                        ..Default::default()
                    })
                    .collect())
            }),
        )
        .unwrap()
    }

    /// section 0
    ///   leaf 1
    ///   section 2
    ///     leaf 3
    ///     leaf 4
    ///   leaf 5
    /// section 6
    ///   leaf 7
    const TREE: &[(usize, bool)] = &[
        (0, true),
        (1, false),
        (1, true),
        (2, false),
        (2, false),
        (1, false),
        (0, true),
        (1, false),
    ];

    fn visible(screen: &Screen) -> (Vec<usize>, Vec<usize>) {
        let down = screen.visible_items().collect::<Vec<_>>();
        let mut up = screen.visible_items_rev().collect::<Vec<_>>();
        up.reverse();
        (down, up)
    }

    /// Walking up is as lazy as walking down, and has to reach the same items.
    #[test]
    fn walks_the_tree_both_ways() {
        let mut screen = tree_screen(TREE, (80, 20));
        assert_eq!(visible(&screen), ((0..8).collect(), (0..8).collect()));

        // An inner section shuts, and is stepped over from either side.
        screen.collapsed.insert(2);
        assert_eq!(
            visible(&screen),
            (vec![0, 1, 2, 5, 6, 7], vec![0, 1, 2, 5, 6, 7])
        );
        assert_eq!(Some(2), screen.prev_visible(5));
        assert_eq!(Some(5), screen.next_visible(2));
        assert!(!screen.is_visible(3));

        // Its outer section shuts over it: the outermost stands in for both.
        screen.collapsed.insert(0);
        assert_eq!(visible(&screen), (vec![0, 6, 7], vec![0, 6, 7]));
        assert_eq!(Some(0), screen.prev_visible(6));
        assert_eq!(Some(6), screen.next_visible(0));
        assert!(!screen.is_visible(2));
    }

    /// Scrolling down over a collapsed section and back up again lands where
    /// it started.
    #[test]
    fn scrolls_back_over_a_collapsed_section() {
        let mut screen = tree_screen(TREE, (80, 4));
        screen.collapsed.insert(2);

        screen.scroll_view_down(3);
        assert_eq!(5, screen.scroll.item_anchor);

        screen.scroll_view_up(3);
        assert_eq!(0, screen.scroll.item_anchor);
        assert_eq!(0, screen.scroll.offset);
    }

    fn laid_out_count(screen: &Screen) -> usize {
        screen
            .item_heights
            .borrow()
            .iter()
            .filter(|height| height.is_some())
            .count()
    }

    /// Items are laid out on demand, so a long list costs no more than a short
    /// one until something actually scrolls down to it.
    #[test]
    fn only_lays_out_what_it_reaches() {
        let mut screen = screen_of(10_000, (80, 20));
        assert!(laid_out_count(&screen) < 50, "{}", laid_out_count(&screen));

        screen.scroll_view_down(100);
        assert!(laid_out_count(&screen) < 150, "{}", laid_out_count(&screen));
    }

    /// The viewport is anchored to an item, so jumping to the bottom lays out
    /// what it lands on rather than everything it skipped over.
    #[test]
    fn jumping_to_the_bottom_skips_the_middle() {
        let mut screen = screen_of(10_000, (80, 20));

        screen.move_cursor_to_bottom();

        assert_eq!(9_999, screen.cursor);
        assert!(laid_out_count(&screen) < 50, "{}", laid_out_count(&screen));
    }

    /// Scrolling is allowed a couple of lines past the content, so on a screen
    /// the content doesn't fill, its center lands past the last line.
    #[test]
    fn recenter_cursor_on_a_screen_the_content_doesnt_fill() {
        let mut screen = screen_of(3, (80, 20));

        screen.scroll_view_down(2);
        screen.refresh().unwrap();

        assert_eq!(2, screen.cursor);
    }

    use crate::git::diff::{Diff, DiffType};
    use crate::items::RenderedRow;

    const DIFF: &str = "diff --git a/f.rs b/f.rs\n\
                        index 1..2 100644\n\
                        --- a/f.rs\n\
                        +++ b/f.rs\n\
                        @@ -1,3 +1,3 @@\n\
                        \x20keep\n\
                        -gone\n\
                        +added\n";

    /// A screen whose diff renders one row per content line, or two when the
    /// `wide` feature is on — standing in for a renderer laying the same diff
    /// out differently.
    fn rendered_screen(rows_per_line: &'static str) -> Screen {
        rendered_screen_of_lines(rows_per_line, 3, 40)
    }

    fn rendered_screen_of_lines(rows_per_line: &'static str, lines: usize, height: u16) -> Screen {
        let diff = Rc::new(Diff {
            text: DIFF.to_string(),
            diff_type: DiffType::WorkdirToIndex,
            file_diffs: crate::gitu_diff::Parser::new(DIFF).parse_diff().unwrap(),
            commit: None,
        });

        Screen::new(
            Arc::new(init_test_config().unwrap()),
            RenderParams {
                size: (80, height),
                features: Rc::from([]),
                context: None,
            },
            Box::new(move |params: RenderParams| {
                let rows = if params.features.iter().any(|f| f == rows_per_line) {
                    2
                } else {
                    1
                };
                Ok(diff_items(&diff, rows, lines))
            }),
        )
        .unwrap()
    }

    /// Items for the diff: a file, a hunk, then each content line drawn over
    /// `rows` rows (the extra ones nesting, as wrapped rows do).
    fn diff_items(diff: &Rc<Diff>, rows: usize, lines: usize) -> Vec<Item> {
        let mut items = vec![
            Item {
                depth: 0,
                data: ItemData::Delta {
                    diff: Rc::clone(diff),
                    file_i: 0,
                    commit: None,
                },
                ..Default::default()
            },
            Item {
                depth: 1,
                data: ItemData::Hunk {
                    diff: Rc::clone(diff),
                    file_i: 0,
                    hunk_i: 0,
                },
                ..Default::default()
            },
        ];

        for line_i in 0..lines {
            for row in 0..rows {
                let rendered: RenderedRow =
                    vec![(format!("line {line_i} row {row}"), Style::new())];
                items.push(Item {
                    depth: if row == 0 { 2 } else { 3 },
                    unselectable: row > 0,
                    data: if row == 0 {
                        ItemData::HunkLine {
                            diff: Rc::clone(diff),
                            file_i: 0,
                            hunk_i: 0,
                            line_i,
                            line_range: 0..0,
                            line_indices: vec![line_i],
                        }
                    } else {
                        ItemData::default()
                    },
                    rendered: Some(Rc::new(rendered)),
                    ..Default::default()
                });
            }
        }
        items
    }

    fn selected_line(screen: &Screen) -> Option<usize> {
        match screen.get_selected_item().data {
            ItemData::HunkLine { line_i, .. } => Some(line_i),
            _ => None,
        }
    }

    /// The view stops once the last page is on screen, but the cursor does not:
    /// it goes on to the end of the content, so holding the page key arrives at
    /// the last line rather than at whichever one it was on when the view ran
    /// out of room.
    #[test]
    fn paging_on_past_the_last_page_reaches_the_last_line() {
        let mut screen = rendered_screen_of_lines("wide", 50, 20);

        for _ in 0..10 {
            screen.full_page_down();
        }

        assert_eq!(selected_line(&screen), Some(49));
    }

    #[test]
    fn a_rerender_keeps_the_cursor_on_the_same_diff_line() {
        let mut screen = rendered_screen("wide");
        screen.select_matching(|data| matches!(data, ItemData::HunkLine { line_i: 2, .. }));
        assert_eq!(selected_line(&screen), Some(2));

        // Every row below the first now has a second row above where the cursor
        // was, so keeping its line number would land somewhere else entirely.
        screen.features = Rc::from(["wide".to_string()]);
        screen.refresh_keeping_position().unwrap();

        assert_eq!(selected_line(&screen), Some(2));
    }

    #[test]
    fn a_plain_refresh_keeps_only_the_place_in_the_list() {
        // The contrast: `refresh` is right for a changed diff, but it is the
        // place in the list that it holds on to, not the diff line.
        let mut screen = rendered_screen("wide");
        screen.select_matching(|data| matches!(data, ItemData::HunkLine { line_i: 2, .. }));

        screen.features = Rc::from(["wide".to_string()]);
        screen.refresh().unwrap();

        assert_ne!(selected_line(&screen), Some(2));
    }
}
