use crate::Res;
use crate::config::Config;
use crate::error::Error;
use crate::git::diff::Diff;
use crate::highlight;
use crate::item_data::ItemData;
use crate::item_data::Ref;
use crate::rebase_todo::{RebaseTodo, TodoAction, TodoEntry};
use crate::style::Modifier;
use crate::style::Style;
use git2::Oid;
use git2::Repository;
use regex::Regex;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::iter;
use std::rc::Rc;
use std::time::SystemTime;

pub type ItemId = u64;

/// A pre-rendered line as owned styled spans (e.g. one row of a renderer's output).
pub(crate) type RenderedRow = Vec<(String, Style)>;

#[derive(Default, Clone, Debug)]
pub(crate) struct Item {
    pub(crate) id: ItemId,
    pub(crate) default_collapsed: bool,
    pub(crate) depth: usize,
    pub(crate) unselectable: bool,
    pub(crate) data: ItemData,
    /// Pre-rendered styled spans (e.g. a diff row from an external renderer),
    /// used verbatim by [`crate::ui::item::layout_item`] in place of `data`.
    pub(crate) rendered: Option<Rc<RenderedRow>>,
}

/// Build the items for a diff. When the configured renderer speaks the OSC-1717
/// protocol, the diff is rendered freely by it (e.g. delta, side-by-side) and its
/// styled rows drive the content lines; otherwise the built-in items are used
/// (styled per-hunk by [`highlight`], including a `--color-only` renderer).
pub(crate) fn create_diff_items(
    config: &Config,
    width: usize,
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
) -> Vec<Item> {
    if config.general.diff_renderer.enabled
        && let Some(items) = create_rendered_diff_items(
            config,
            width,
            diff,
            depth,
            default_collapsed,
            commit.clone(),
        )
    {
        return items;
    }
    create_builtin_diff_items(diff, depth, default_collapsed, commit).collect()
}

/// Render the diff through the OSC-1717 renderer and lay its rows out under
/// gitu's own file/hunk structure (so collapse, navigation and file/hunk/line
/// staging keep working). Each rendered content row is mapped back to its
/// content-line coordinates via its metadata; decoration rows are dropped.
/// Returns `None` (fall back to built-in) if the renderer doesn't speak the
/// protocol.
fn create_rendered_diff_items(
    config: &Config,
    width: usize,
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
) -> Option<Vec<Item>> {
    use std::collections::HashMap;

    let output = crate::diff_renderer::run(
        &config.general.diff_renderer.command,
        Some(&diff.text),
        width,
        None,
    )?;
    let parsed = crate::diff_renderer::parse_ansi_lines(&output);
    parsed.protocol_version?; // Not an OSC-1717 renderer: fall back to built-in.

    use crate::diff_renderer::LineKind;

    // Per hunk: the renderer's own hunk-header rows (`h`) to display in place of
    // `@@`, and the content-line items. `f` (file-header) rows are dropped — gitu
    // draws its own file header — as are the renderer's blank spacer rows.
    let mut headers_by_hunk: HashMap<(usize, usize), Vec<RenderedRow>> = HashMap::new();
    let mut content_by_hunk: HashMap<(usize, usize), Vec<Item>> = HashMap::new();

    for line in &parsed.lines {
        let Some(first) = line.records.first() else {
            continue; // Un-annotated decoration (dividers): dropped.
        };
        match first.kind {
            LineKind::FileHeader | LineKind::Commit => {}
            LineKind::HunkHeader => {
                if line.text.trim().is_empty() {
                    continue;
                }
                if let Some(key) = crate::diff_renderer::resolve_hunk(diff, first) {
                    headers_by_hunk
                        .entry(key)
                        .or_default()
                        .push(rendered_spans(line));
                }
            }
            LineKind::Context | LineKind::Added | LineKind::Deleted => {
                let Some((file_i, hunk_i, line_i)) =
                    crate::diff_renderer::resolve_line(diff, first)
                else {
                    continue;
                };
                // A fused side-by-side change row also carries its replacement, so
                // stage every record in the same hunk.
                let line_indices = line
                    .records
                    .iter()
                    .filter_map(|meta| crate::diff_renderer::resolve_line(diff, meta))
                    .filter(|(f, h, _)| (*f, *h) == (file_i, hunk_i))
                    .map(|(_, _, li)| li)
                    .collect();
                let hunk_hash = hash([diff.file_diff_header(file_i), diff.hunk(file_i, hunk_i)]);
                let line_range = highlight::line_range_iterator(diff.hunk_content(file_i, hunk_i))
                    .nth(line_i)
                    .map(|(range, _)| range)
                    .unwrap_or_default();
                content_by_hunk
                    .entry((file_i, hunk_i))
                    .or_default()
                    .push(Item {
                        id: hunk_hash,
                        depth: depth + 2,
                        unselectable: matches!(first.kind, LineKind::Context),
                        data: ItemData::HunkLine {
                            diff: Rc::clone(diff),
                            file_i,
                            hunk_i,
                            line_i,
                            line_range,
                            line_indices,
                        },
                        rendered: Some(Rc::new(rendered_spans(line))),
                        ..Default::default()
                    });
            }
        }
    }

    let mut items = Vec::new();
    for (file_i, file_diff) in diff.file_diffs.iter().enumerate() {
        items.push(Item {
            id: hash(diff.file_diff_header(file_i)),
            default_collapsed,
            depth,
            data: ItemData::Delta {
                diff: Rc::clone(diff),
                file_i,
                commit: commit.clone(),
            },
            ..Default::default()
        });
        for hunk_i in 0..file_diff.hunks.len() {
            let hunk_hash = hash([diff.file_diff_header(file_i), diff.hunk(file_i, hunk_i)]);
            let hunk = ItemData::Hunk {
                diff: Rc::clone(diff),
                file_i,
                hunk_i,
            };
            // Render the renderer's own hunk header: its first row is the
            // selectable/collapsible Hunk anchor; the rest nest under it (so a
            // collapsed hunk shows just the anchor). Fall back to gitu's `@@` when
            // the renderer emitted no header for this hunk.
            let mut header_rows = headers_by_hunk
                .remove(&(file_i, hunk_i))
                .unwrap_or_default()
                .into_iter();
            items.push(Item {
                id: hunk_hash,
                depth: depth + 1,
                data: hunk,
                rendered: header_rows.next().map(Rc::new),
                ..Default::default()
            });
            for extra in header_rows {
                items.push(Item {
                    id: hunk_hash,
                    depth: depth + 2,
                    unselectable: true,
                    rendered: Some(Rc::new(extra)),
                    ..Default::default()
                });
            }
            if let Some(rows) = content_by_hunk.remove(&(file_i, hunk_i)) {
                items.extend(rows);
            }
        }
    }
    Some(items)
}

/// The renderer's styled runs for a line, as owned `(text, style)` spans with
/// tabs expanded (matching the built-in hunk-line rendering).
fn rendered_spans(line: &crate::diff_renderer::ParsedLine) -> RenderedRow {
    line.runs
        .iter()
        .map(|(range, style)| (line.text[range.clone()].replace('\t', "    "), *style))
        .collect()
}

fn create_builtin_diff_items(
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
) -> impl Iterator<Item = Item> + '_ {
    diff.file_diffs
        .iter()
        .enumerate()
        .flat_map(move |(file_i, file_diff)| {
            iter::once(Item {
                id: hash(diff.file_diff_header(file_i)),
                default_collapsed,
                depth,
                data: ItemData::Delta {
                    diff: Rc::clone(diff),
                    file_i,
                    commit: commit.clone(),
                },
                ..Default::default()
            })
            .chain(file_diff.hunks.iter().cloned().enumerate().flat_map(
                move |(hunk_i, _hunk)| {
                    create_hunk_items(Rc::clone(diff), file_i, hunk_i, depth + 1)
                },
            ))
        })
}

fn create_hunk_items(
    diff: Rc<Diff>,
    file_i: usize,
    hunk_i: usize,
    depth: usize,
) -> impl Iterator<Item = Item> {
    let hunk_hash = hash([diff.file_diff_header(file_i), diff.hunk(file_i, hunk_i)]);
    iter::once(Item {
        id: hunk_hash,
        depth,
        data: ItemData::Hunk {
            diff: Rc::clone(&diff),
            file_i,
            hunk_i,
        },
        ..Default::default()
    })
    .chain(format_diff_hunk_items(
        diff,
        file_i,
        hunk_i,
        depth + 1,
        hunk_hash,
    ))
}

fn format_diff_hunk_items(
    diff: Rc<Diff>,
    file_i: usize,
    hunk_i: usize,
    depth: usize,
    hunk_hash: u64,
) -> Vec<Item> {
    let hunk_content = diff.hunk_content(file_i, hunk_i);

    highlight::line_range_iterator(hunk_content)
        .enumerate()
        .map(|(line_index, (line_range, line))| {
            Item {
                id: hunk_hash,
                // line is marked unselectable if it starts with a space character
                unselectable: line.starts_with(' '),
                depth,
                data: ItemData::HunkLine {
                    diff: Rc::clone(&diff),
                    file_i,
                    hunk_i,
                    line_i: line_index,
                    line_range,
                    line_indices: vec![line_index],
                },
                ..Default::default()
            }
        })
        .collect()
}

pub(crate) fn stash_list(repo: &Repository, limit: usize) -> Res<Vec<Item>> {
    Ok(repo
        .reflog("refs/stash")
        .map_err(Error::StashList)?
        .iter()
        .enumerate()
        .map(|(i, stash)| -> Res<Item> {
            let stash_id = stash.id_new();
            let stash_ref = format!("stash@{{{}}}", i);
            Ok(Item {
                id: hash(stash_id),
                depth: 1,
                data: ItemData::Stash {
                    message: stash.message().unwrap_or("").to_string(),
                    stash_ref,
                    id: i,
                },
                ..Default::default()
            })
        })
        .map(|result| match result {
            Ok(item) => item,
            Err(err) => {
                let err = err.to_string();
                Item {
                    id: hash(&err),
                    data: ItemData::Error(err),
                    ..Default::default()
                }
            }
        })
        .take(limit)
        .collect::<Vec<_>>())
}

fn short_age(time: git2::Time) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since_epoch| since_epoch.as_secs() as i64);

    // A commit dated in the future reads as brand new, rather than as a negative age.
    let age = (now - time.seconds()).max(0);

    if age < HOUR {
        format!("{}m", age / MINUTE)
    } else if age < DAY {
        format!("{}h", age / HOUR)
    } else if age < WEEK {
        format!("{}d", age / DAY)
    } else if age < MONTH {
        format!("{}w", age / WEEK)
    } else if age < YEAR {
        format!("{}M", age / MONTH)
    } else {
        format!("{}y", age / YEAR)
    }
}

pub(crate) fn log(
    repo: &Repository,
    limit: usize,
    rev: Option<Oid>,
    msg_regex: Option<Regex>,
) -> Res<Vec<Item>> {
    let mut revwalk = repo.revwalk().map_err(Error::ReadLog)?;
    if let Some(r) = rev {
        revwalk.push(r).map_err(Error::ReadLog)?;
    } else if revwalk.push_head().is_err() {
        return Ok(vec![]);
    }

    let references = commit_refs(repo)?;

    let items: Vec<Item> = revwalk
        .map(|oid_result| -> Res<Option<Item>> {
            let oid = oid_result.map_err(Error::ReadLog)?;
            let commit = repo.find_commit(oid).map_err(Error::ReadLog)?;

            let short_id = commit.as_object().short_id().map_err(Error::ReadOid)?;
            let short_id = String::from_utf8_lossy(&short_id).to_string();

            if let Some(re) = &msg_regex
                && !re.is_match(commit.message().unwrap_or(""))
            {
                return Ok(None);
            }

            let associated_references: Vec<_> = references
                .iter()
                .filter(|(commit, _)| commit.id() == oid)
                .map(|(_, reference)| reference.clone())
                .collect();

            let data = ItemData::Commit {
                oid: oid.to_string(),
                short_id,
                associated_references,
                summary: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                age: short_age(commit.author().when()),
            };

            Ok(Some(Item {
                id: hash(oid),
                depth: 1,
                data,
                ..Default::default()
            }))
        })
        .filter_map(|result| match result {
            Ok(item) => item,
            Err(err) => {
                let err = err.to_string();
                Some(Item {
                    id: hash(&err),
                    data: ItemData::Error(err),
                    ..Default::default()
                })
            }
        })
        .take(limit)
        .collect();

    if items.is_empty() {
        Ok(vec![Item {
            data: ItemData::Raw("No commits found".to_string()),
            ..Default::default()
        }])
    } else {
        Ok(items)
    }
}

/// The refs pointing at commits, as the log view annotates them with.
fn commit_refs(repo: &Repository) -> Res<Vec<(git2::Commit<'_>, Ref)>> {
    Ok(repo
        .references()
        .map_err(Error::ReadLog)?
        .filter_map(Result::ok)
        .filter_map(
            |reference| match (reference.peel_to_commit(), reference.shorthand()) {
                (Ok(target), Some(name)) => {
                    if name.ends_with("/HEAD") || name.starts_with("prefetch/remotes/") {
                        return None;
                    }

                    let name = name.to_owned();

                    let ref_kind = if reference.is_remote() {
                        Ref::Remote(name)
                    } else if reference.is_tag() {
                        Ref::Tag(name)
                    } else {
                        Ref::Head(name)
                    };

                    Some((target, ref_kind))
                }
                _ => None,
            },
        )
        .collect())
}

/// Build the log view from the output of the configured log command
/// (`general.log_renderer`), which prints the commits however it likes — several
/// rows each, `--stat`, `--graph`. gitu substitutes the command's `{commit}`
/// token with git format directives that emit an OSC-1717 commit record, so each
/// commit's rows are known: the first non-blank one becomes the selectable
/// `Commit` item and the rest nest under it, keeping a commit one unit for
/// navigation, folding and the ops that act on a commit.
///
/// Returns `None` (fall back to the built-in log) if the command fails or its
/// output carries no commit records.
pub(crate) fn rendered_log(
    config: &Config,
    repo: &Repository,
    width: usize,
    limit: usize,
    rev: Option<Oid>,
    msg_regex: Option<&Regex>,
) -> Option<Vec<Item>> {
    let mut args = Vec::new();
    // "No limit" is `u32::MAX`, which git rejects as not an integer; leaving the
    // option off is what an unrepresentable limit means anyway.
    if let Ok(limit) = i32::try_from(limit) {
        args.push(format!("-n{limit}"));
    }
    if let Some(regex) = msg_regex {
        args.push("--extended-regexp".into());
        args.push(format!("--grep={regex}"));
    }
    args.push(rev.map_or_else(|| "HEAD".to_string(), |oid| oid.to_string()));

    let blocks = rendered_commits(config, repo, width, &args)?;
    let references = commit_refs(repo).ok()?;
    Some(
        blocks
            .iter()
            .flat_map(|block| commit_block_items(repo, &references, block))
            .collect(),
    )
}

/// A run of rendered log rows belonging to one commit.
pub(crate) struct RenderedCommit {
    /// `None` for rows preceding the first commit the command printed.
    oid: Option<String>,
    rows: Vec<Rc<RenderedRow>>,
}

/// Run the configured log command with `args` appended, and group its rows per
/// commit. `None` if the command fails or emits no commit records.
fn rendered_commits(
    config: &Config,
    repo: &Repository,
    width: usize,
    args: &[String],
) -> Option<Vec<RenderedCommit>> {
    let command: Vec<String> = config
        .general
        .log_renderer
        .command
        .iter()
        .map(|arg| arg.replace("{commit}", crate::diff_renderer::COMMIT_RECORD_FORMAT))
        .chain(args.iter().cloned())
        .collect();

    let dir = repo.workdir().unwrap_or_else(|| repo.path());
    let output = crate::diff_renderer::run(&command, None, width, Some(dir))?;
    let parsed = crate::diff_renderer::parse_ansi_lines(&output);
    let blocks = crate::diff_renderer::commit_blocks(&parsed.lines);

    if blocks.iter().all(|block| block.commit.is_none()) {
        log::warn!(
            "log command emitted no commit records; is the '{{commit}}' token in its format?"
        );
        return None;
    }

    Some(
        blocks
            .iter()
            .map(|block| RenderedCommit {
                oid: block.commit.map(String::from),
                rows: block
                    .rows
                    .iter()
                    .map(|row| Rc::new(rendered_spans(row)))
                    .collect(),
            })
            .collect(),
    )
}

/// The log command's rows for each commit in `revs`, keyed by oid, for views
/// that lay the commits out themselves (the interactive rebase todo). Empty when
/// no log command is configured or it fails; the caller renders those commits.
pub(crate) fn rendered_commit_rows(
    config: &Config,
    repo: &Repository,
    width: usize,
    revs: &str,
) -> HashMap<String, Vec<Rc<RenderedRow>>> {
    if !config.general.log_renderer.enabled {
        return HashMap::new();
    }

    rendered_commits(config, repo, width, &[revs.to_string()])
        .unwrap_or_default()
        .into_iter()
        .filter_map(|block| Some((block.oid?, block.rows)))
        .collect()
}

/// One commit's rendered rows as items: blank rows leading the block stay
/// separators, the first non-blank row is the commit itself, and the remaining
/// rows nest under it (so folding the commit hides them).
fn commit_block_items(
    repo: &Repository,
    references: &[(git2::Commit, Ref)],
    block: &RenderedCommit,
) -> Vec<Item> {
    let commit = block
        .oid
        .as_ref()
        .and_then(|oid| Oid::from_str(oid).ok())
        .and_then(|oid| repo.find_commit(oid).ok());

    let Some(commit) = commit else {
        return block.rows.iter().map(|row| row_item(0, 1, row)).collect();
    };

    block_items(hash(commit.id()), &block.rows, |row| Item {
        data: commit_data(&commit, references),
        unselectable: false,
        ..row_item(hash(commit.id()), 1, row)
    })
}

/// Lay a block of rendered rows out under a selectable anchor row (the first
/// non-blank one, built by `anchor`): rows before it are separators at the same
/// depth, rows after it nest one deeper so folding the anchor hides them.
fn block_items(
    id: ItemId,
    rows: &[Rc<RenderedRow>],
    anchor: impl Fn(&Rc<RenderedRow>) -> Item,
) -> Vec<Item> {
    let Some(anchor_i) = rows.iter().position(|row| !is_blank(row)) else {
        return rows.iter().map(|row| row_item(id, 1, row)).collect();
    };

    rows.iter()
        .enumerate()
        .map(|(i, row)| match i.cmp(&anchor_i) {
            Ordering::Less => row_item(id, 1, row),
            Ordering::Greater => row_item(id, 2, row),
            Ordering::Equal => anchor(row),
        })
        .collect()
}

fn is_blank(row: &RenderedRow) -> bool {
    row.iter().all(|(text, _)| text.trim().is_empty())
}

fn row_item(id: ItemId, depth: usize, row: &Rc<RenderedRow>) -> Item {
    Item {
        id,
        depth,
        unselectable: true,
        rendered: Some(Rc::clone(row)),
        ..Default::default()
    }
}

/// Build the interactive rebase todo view: each entry's commit is drawn with the
/// rows the log command gave it (so it looks like the log view), prefixed with
/// the action git will take. Entries that aren't commits (`exec`, `break`, …)
/// show their instruction verbatim.
pub(crate) fn rebase_todo_items(
    config: &Config,
    repo: &Repository,
    todo: &Rc<RefCell<RebaseTodo>>,
    rows_by_commit: &HashMap<String, Vec<Rc<RenderedRow>>>,
) -> Vec<Item> {
    let mut items = Vec::new();

    for (index, entry) in todo.borrow().entries.iter().enumerate() {
        let data = ItemData::RebaseTodo {
            todo: Rc::clone(todo),
            index,
        };
        let (id, keyword, rows) = match entry {
            TodoEntry::Commit { action, oid } => (
                hash(oid),
                Some(*action),
                rows_by_commit
                    .get(oid)
                    .cloned()
                    .unwrap_or_else(|| vec![Rc::new(commit_row(config, repo, oid))]),
            ),
            TodoEntry::Other(instruction) => (
                hash(instruction),
                None,
                vec![Rc::new(vec![(instruction.clone(), Style::new())])],
            ),
        };

        items.extend(block_items(id, &rows, |row| Item {
            data: data.clone(),
            unselectable: false,
            ..row_item(id, 1, &Rc::new(action_prefixed(config, keyword, row)))
        }));
    }

    items
}

/// The anchor row with the entry's action in front of it, so the instruction
/// reads as part of the commit's own line.
fn action_prefixed(
    config: &Config,
    action: Option<TodoAction>,
    row: &Rc<RenderedRow>,
) -> RenderedRow {
    let Some(action) = action else {
        return row.as_ref().clone();
    };

    let mut style = Style::from(&config.style.rebase_todo_action);
    if action == TodoAction::Drop {
        style.add_modifier.insert(Modifier::CROSSED_OUT);
    }

    iter::once((format!("{:<7}", action.keyword()), style))
        .chain(row.iter().cloned())
        .collect()
}

/// A commit as gitu would show it in the built-in log, for when no log command
/// rendered it.
fn commit_row(config: &Config, repo: &Repository, oid: &str) -> RenderedRow {
    let commit = Oid::from_str(oid)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok());
    let Some(commit) = commit else {
        return vec![(oid.to_string(), Style::new())];
    };

    let short_id = commit
        .as_object()
        .short_id()
        .map(|id| String::from_utf8_lossy(&id).to_string())
        .unwrap_or_default();

    vec![
        (format!("{short_id} "), Style::from(&config.style.hash)),
        (commit.summary().unwrap_or("").to_string(), Style::new()),
    ]
}

fn commit_data(commit: &git2::Commit, references: &[(git2::Commit, Ref)]) -> ItemData {
    let short_id = commit
        .as_object()
        .short_id()
        .map(|id| String::from_utf8_lossy(&id).to_string())
        .unwrap_or_default();

    ItemData::Commit {
        oid: commit.id().to_string(),
        short_id,
        associated_references: references
            .iter()
            .filter(|(target, _)| target.id() == commit.id())
            .map(|(_, reference)| reference.clone())
            .collect(),
        summary: commit.summary().unwrap_or("").to_string(),
        author: commit.author().name().unwrap_or("").to_string(),
        age: short_age(commit.author().when()),
    }
}

pub(crate) fn blank_line() -> Item {
    Item {
        depth: 0,
        unselectable: true,
        ..Default::default()
    }
}

pub(crate) fn hash<T: Hash>(x: T) -> ItemId {
    let mut hasher = DefaultHasher::new();
    x.hash(&mut hasher);
    hasher.finish()
}
