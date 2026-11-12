use std::borrow::Cow;

use crate::menu::Menu;
use crate::menu::arg::Arg;
use crate::style::Style;
use crate::ui::layout::opts;
use crate::ui::{self, UiTree, layout_line, layout_span, repeat_chars};
use crate::{
    app::State,
    config::{Config, ReadOnly},
    ops::Op,
};
use itertools::Itertools;
use unicode_width::UnicodeWidthStr;

/// The value column of a keybind table row.
#[derive(Clone)]
enum MenuValue<'a> {
    Text(Cow<'a, str>),
    Arg(&'a Arg),
}

pub(crate) fn layout_menu<'a>(layout: &mut UiTree<'a>, state: &'a State, size: (u16, u16)) {
    let (width, height) = (size.0 as usize, size.1);
    let Some(ref pending) = state.pending_menu else {
        return;
    };

    if pending.is_hidden {
        return;
    }

    if state.picker.is_some() {
        return;
    }

    let config = &state.config;
    let item = state.screens.last().unwrap().get_selected_item();
    let style = &config.style;

    let arg_binds = config.bindings.arg_list(pending).collect::<Vec<_>>();
    let (target_binds, non_target_binds): (Vec<_>, Vec<_>) = config
        .bindings
        .list(&pending.menu)
        .filter(|keybind| !(state.read_only && keybind.op.writes()))
        .partition(|keybind| keybind.op.clone().implementation().is_target_op());
    let target_binds: Vec<_> = target_binds
        .into_iter()
        .filter(|keybind| {
            keybind
                .op
                .clone()
                .implementation()
                .get_action(&item.data)
                .is_some()
        })
        .collect();
    let (menu_binds, non_menu_binds): (Vec<_>, Vec<_>) = non_target_binds
        .into_iter()
        .chunk_by(|bind| &bind.op)
        .into_iter()
        .map(|(op, binds)| {
            let binds: Vec<_> = binds.collect();
            (op, binds)
        })
        .partition(|(op, _binds)| matches!(op, Op::OpenMenu(_)));

    // Feature keys are top-level keys, so they are listed where those are.
    let feature_binds: Vec<_> = if matches!(pending.menu, Menu::Root | Menu::Help) {
        state
            .feature_keys
            .iter()
            .map(|(feature, key)| {
                (
                    key.to_string().into(),
                    MenuValue::Text(feature.as_str().into()),
                )
            })
            .collect()
    } else {
        vec![]
    };

    let separator_style = Style::from(&style.separator);
    // The help's columns are in order of key, so that a key is found as in a dictionary; a menu's
    // keep the order they are configured in.
    let sorted = matches!(pending.menu, Menu::Root | Menu::Help);

    // Says why the ops that write are missing from the menus that list them.
    let read_only_notice = (state.read_only && matches!(pending.menu, Menu::Root | Menu::Help))
        .then(|| {
            let first = match config.general.read_only {
                ReadOnly::Auto => "Read-only (.jj found):",
                ReadOnly::On | ReadOnly::Off => "Read-only:",
            };
            format!("{first} commands that change the repository are hidden")
        });

    let heading = |text: Cow<'a, str>| Line::Heading(text, Style::from(&style.menu.heading));
    let binds = |rows: Vec<(Cow<'a, str>, MenuValue<'a>)>| rows.into_iter().map(Line::Bind);
    let mut columns: Vec<Vec<Line<'a>>> = vec![];

    // Column 1: Main menu commands
    if !non_menu_binds.is_empty() {
        let title = match pending.menu {
            Menu::Root | Menu::Help => "Commands".into(),
            menu => menu.to_string(),
        };
        let rows = non_menu_binds
            .into_iter()
            .map(|(op, binds)| {
                (
                    binds.iter().map(|bind| bind.raw.as_str()).join("/").into(),
                    MenuValue::Text(op.clone().implementation().display(state).into()),
                )
            })
            .collect();
        columns.push(
            std::iter::once(heading(title.into()))
                .chain(binds(in_order(sorted, rows)))
                .collect(),
        );
    }

    // Column 2: Submenus, and the keys toggling renderer features
    if !menu_binds.is_empty() || !feature_binds.is_empty() {
        let mut column = vec![];
        if !menu_binds.is_empty() {
            let rows = menu_binds
                .into_iter()
                .map(|(op, binds)| {
                    let Op::OpenMenu(menu) = op else {
                        unreachable!();
                    };
                    (
                        binds.iter().map(|bind| bind.raw.as_str()).join("/").into(),
                        MenuValue::Text(menu.to_string().into()),
                    )
                })
                .collect();
            column.push(heading("Submenus".into()));
            column.extend(binds(in_order(sorted, rows)));
        }
        if !feature_binds.is_empty() {
            if !column.is_empty() {
                column.push(Line::Blank);
            }
            column.push(heading("Hotkeys".into()));
            column.extend(binds(in_order(sorted, feature_binds)));
        }
        columns.push(column);
    }

    // Column 3: Target commands and arguments
    let mut column = vec![];
    if !target_binds.is_empty() {
        column.push(heading(item.data.kind().into()));
        column.extend(binds(
            target_binds
                .into_iter()
                .map(|bind| {
                    (
                        bind.raw.as_str().into(),
                        MenuValue::Text(bind.op.clone().implementation().display(state).into()),
                    )
                })
                .collect(),
        ));
    }
    if let Some(notice) = read_only_notice {
        if !column.is_empty() {
            column.push(Line::Blank);
        }
        column.push(Line::Text(notice.into(), Style::from(&style.error_msg)));
    }
    if !arg_binds.is_empty() {
        column.push(heading("Arguments".into()));
        column.extend(binds(
            arg_binds
                .into_iter()
                .map(|bind| {
                    let Op::ToggleArg(name) = &bind.op else {
                        unreachable!();
                    };
                    let arg = pending.args.get(name.as_str()).unwrap();
                    (bind.raw.as_str().into(), MenuValue::Arg(arg))
                })
                .collect(),
        ));
    }
    if !column.is_empty() {
        columns.push(column);
    }

    // The panel takes all but one row of the terminal: the separator, the lines, and, if anything
    // is cut, a row saying how much.
    let max_lines = (height as usize).saturating_sub(2).max(1);
    let (shown, cut) = match fit(&columns, max_lines, width) {
        (_, 0) => fit(&columns, max_lines, width),
        _ => fit(&columns, max_lines.saturating_sub(1).max(1), width),
    };

    layout.col(opts(), |layout| {
        ui::repeat_chars(layout, width, ui::DASHES, separator_style);

        layout.row(opts().gap(3).pad(1), |layout| {
            for lines in shown {
                layout_lines(layout, config, lines);
            }
        });

        if cut > 0 {
            layout_line(
                layout,
                format!(" … {cut} more: make the window larger").into(),
                Style::from(&style.menu.heading),
            );
        }
    });
}

/// A line of a menu column.
#[derive(Clone)]
enum Line<'a> {
    Heading(Cow<'a, str>, Style),
    Text(Cow<'a, str>, Style),
    Bind((Cow<'a, str>, MenuValue<'a>)),
    Blank,
}

impl Line<'_> {
    /// Its width, given the width its table gives keys.
    fn width(&self, key_width: usize) -> usize {
        match self {
            Line::Heading(text, _) | Line::Text(text, _) => text.width(),
            Line::Bind((_, value)) => key_width + value.width(),
            Line::Blank => 0,
        }
    }
}

impl MenuValue<'_> {
    fn width(&self) -> usize {
        match self {
            MenuValue::Text(text) => text.width(),
            MenuValue::Arg(arg) => arg.display.width() + arg.get_cli_token().width() + 3,
        }
    }
}

/// The width each line gives its key: the widest key of its table and a space. A table is a run
/// of keys between headings or blank lines.
fn key_widths(lines: &[Line]) -> Vec<usize> {
    let mut widths = vec![0; lines.len()];
    let mut start = 0;
    for end in 0..=lines.len() {
        if end == lines.len() || !matches!(lines[end], Line::Bind(_)) {
            let width = lines[start..end]
                .iter()
                .filter_map(|line| match line {
                    Line::Bind((key, _)) => Some(key.width() + 1),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            widths[start..end].fill(width);
            start = end + 1;
        }
    }
    widths
}

/// The narrowest a sub-column of long lines is laid out at, wrapping them.
const MIN_WRAPPED_WIDTH: usize = 30;

/// `columns` flowed into sub-columns of at most `max_lines` lines, as many of them as fit in
/// `width`, and how many keys were left out. A column taller than `max_lines` continues in the
/// next sub-column, below the row of headings, and a heading is not left last.
fn fit<'a>(
    columns: &[Vec<Line<'a>>],
    max_lines: usize,
    width: usize,
) -> (Vec<Vec<Line<'a>>>, usize) {
    let mut sub_columns: Vec<Vec<Line>> = vec![];
    for column in columns {
        let mut current: Vec<Line> = vec![];
        for line in column.iter().cloned() {
            if current.len() == max_lines {
                // A continuation starts below the row of headings, unless it starts with a
                // heading itself, carried from the end of the sub-column before.
                let carried = match current.last() {
                    Some(Line::Heading(..)) if current.len() > 1 => current.pop(),
                    _ => None,
                };
                sub_columns.push(std::mem::take(&mut current));
                current.push(carried.unwrap_or(Line::Blank));
            }
            if current.len() == 1
                && matches!(current[0], Line::Blank)
                && matches!(line, Line::Blank)
            {
                continue;
            }
            current.push(line);
        }
        if !current.is_empty() {
            sub_columns.push(current);
        }
    }

    // The row of sub-columns has a column of padding at each end and 3 between sub-columns.
    let mut used = 2;
    let mut shown = vec![];
    let mut cut = 0;
    for lines in sub_columns {
        let keys = key_widths(&lines);
        let needed = lines
            .iter()
            .zip(keys)
            .map(|(line, keys)| line.width(keys))
            .max()
            .unwrap_or(0);
        let gap = if shown.is_empty() { 0 } else { 3 };
        // A long line wraps within its sub-column, so a sub-column fits if the rest of its lines
        // do, in what is left.
        if cut == 0 && used + gap + needed.min(MIN_WRAPPED_WIDTH) <= width {
            used += gap + needed;
            shown.push(lines);
        } else {
            cut += lines
                .iter()
                .filter(|line| matches!(line, Line::Bind(_)))
                .count();
        }
    }
    (shown, cut)
}

/// A sub-column of lines, its keys aligned.
fn layout_lines<'a>(layout: &mut UiTree<'a>, config: &Config, lines: Vec<Line<'a>>) {
    const SPACES: &str = "                                                                ";
    let key_style = Style::from(&config.style.menu.key);
    let keys = key_widths(&lines);

    layout.col(opts(), |layout| {
        for (line, keys) in lines.into_iter().zip(keys) {
            match line {
                Line::Heading(text, style) | Line::Text(text, style) => {
                    layout_line(layout, text, style)
                }
                Line::Blank => layout_line(layout, " ".into(), Style::new()),
                Line::Bind((key, value)) => {
                    let padding = keys - key.width();
                    layout.row(opts(), |layout| {
                        layout_line(layout, key, key_style);
                        repeat_chars(layout, padding, SPACES, Style::new());
                        layout.row(opts(), |layout| layout_value(layout, config, value));
                    });
                }
            }
        }
    });
}

fn layout_value<'a>(layout: &mut UiTree<'a>, config: &Config, value: MenuValue<'a>) {
    match value {
        MenuValue::Text(text) => layout_span(layout, (text, Style::new())),
        MenuValue::Arg(arg) => {
            layout_span(layout, (arg.display.into(), Style::new()));
            layout_span(layout, (" (".into(), Style::new()));
            layout_span(
                layout,
                (
                    arg.get_cli_token().into(),
                    if arg.is_active() {
                        Style::from(&config.style.menu.active_arg)
                    } else {
                        Style::from(&config.style.menu.inactive_arg)
                    },
                ),
            );
            layout_span(layout, (")".into(), Style::new()));
        }
    }
}

/// `rows` in order of key if `sorted`: ignoring case, with a lowercase key before its capital.
fn in_order<'a>(
    sorted: bool,
    mut rows: Vec<(Cow<'a, str>, MenuValue<'a>)>,
) -> Vec<(Cow<'a, str>, MenuValue<'a>)> {
    if sorted {
        rows.sort_by_key(|(key, _)| (key.to_lowercase(), key.chars().any(char::is_uppercase)));
    }
    rows
}
