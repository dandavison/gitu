use crate::Res;
use crate::config::Config;
use crate::error::Error;
use crate::git::diff::Diff;
use crate::highlight;
use crate::item_data::ItemData;
use crate::item_data::Ref;
use crate::rebase_todo::{RebaseTodo, TodoAction, TodoEntry};
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

/// Everything a screen's rows depend on besides the repository: the viewport
/// they must fit, and the renderer features in force.
#[derive(Default, Clone)]
pub(crate) struct RenderParams {
    pub size: (u16, u16),
    /// Named renderer features overlaying the user's own configuration, chosen
    /// in-session. Empty means their configuration alone.
    pub features: Rc<[String]>,
    /// How much of a file to ask git for around each change (`-U8`, `-W`),
    /// chosen in-session. `None` is git's own default.
    pub context: Option<Rc<str>>,
}

impl RenderParams {
    /// Columns to render a row into: the viewport width less the 1-char gutter,
    /// and one more so a renderer that pads rows to full width (delta
    /// side-by-side) doesn't reach the edge, where the overflow guard would clip
    /// it.
    pub(crate) fn width(&self) -> usize {
        (self.size.0 as usize).saturating_sub(2)
    }
}

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
    params: &RenderParams,
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
) -> Vec<Item> {
    if config.general.diff_renderer.enabled
        && let Some(items) = create_rendered_diff_items(
            config,
            params,
            diff,
            depth,
            default_collapsed,
            commit.clone(),
        )
    {
        return items;
    }
    create_builtin_diff_items(
        diff,
        depth,
        default_collapsed,
        commit,
        config.general.visit_context_lines,
    )
    .collect()
}

/// Render the diff through the OSC-1717 renderer and lay its rows out under
/// gitu's own file/hunk structure (so collapse, navigation and file/hunk/line
/// staging keep working). Each rendered content row is mapped back to its
/// content-line coordinates via its metadata; decoration rows are dropped.
/// Returns `None` (fall back to built-in) if the renderer doesn't speak the
/// protocol.
fn create_rendered_diff_items(
    config: &Config,
    params: &RenderParams,
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
) -> Option<Vec<Item>> {
    let output = crate::diff_renderer::run(
        &config.general.diff_renderer.command,
        Some(&diff.text),
        params,
        None,
    )?;
    let parsed = crate::diff_renderer::parse_ansi_lines(&output);
    parsed.protocol_version?; // Not an OSC-1717 renderer: fall back to built-in.

    // The handshake says the renderer speaks the protocol, not that it said
    // anything with it: without content records there is nothing to lay out.
    let has_hunks = diff.file_diffs.iter().any(|file| !file.hunks.is_empty());
    let annotated_a_line = parsed
        .lines
        .iter()
        .flat_map(|line| &line.records)
        .any(|record| record.kind.is_content());
    if has_hunks && !annotated_a_line {
        return None;
    }

    Some(rendered_diff_items(
        diff,
        &parsed.lines,
        depth,
        default_collapsed,
        commit,
        config.general.visit_context_lines,
    ))
}

/// Lay the renderer's rows out under gitu's file/hunk structure. Split from
/// [`create_rendered_diff_items`] so the mapping can be exercised on rows
/// without running a renderer.
fn rendered_diff_items(
    diff: &Rc<Diff>,
    lines: &[crate::diff_renderer::ParsedLine],
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
    visit_context_lines: bool,
) -> Vec<Item> {
    use crate::diff_renderer::LineKind;

    // Per hunk: the renderer's own hunk-header rows (`h`) to display in place of
    // `@@`, and the content-line items. `f` (file-header) rows are dropped — gitu
    // draws its own file header — as are the renderer's blank spacer rows.
    let mut headers_by_hunk: HashMap<(usize, usize), Vec<RenderedRow>> = HashMap::new();
    let mut content_by_hunk: HashMap<(usize, usize), Vec<Item>> = HashMap::new();
    // The content line the previous row belonged to; a row repeating it is a
    // wrapped continuation of it. Cleared by every other kind of row, so only a
    // consecutive run counts as one line.
    let mut last_content: Option<(usize, usize, usize)> = None;

    for line in lines {
        let Some(first) = line.records.first() else {
            last_content = None;
            continue; // Un-annotated decoration (dividers): dropped.
        };
        match first.kind {
            LineKind::FileHeader | LineKind::Commit => last_content = None,
            LineKind::HunkHeader => {
                last_content = None;
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
                let rows = content_by_hunk.entry((file_i, hunk_i)).or_default();

                // A renderer that wraps a long line emits a row per screen line,
                // re-emitting the same record on each. They are one diff line, so
                // the first takes the cursor and the rest nest under it.
                let continues_previous = last_content
                    .replace((file_i, hunk_i, line_i))
                    .is_some_and(|previous| previous == (file_i, hunk_i, line_i));
                if continues_previous {
                    rows.push(Item {
                        id: hunk_hash,
                        depth: depth + 3,
                        unselectable: true,
                        rendered: Some(Rc::new(rendered_spans(line))),
                        ..Default::default()
                    });
                    continue;
                }

                let line_range = highlight::line_range_iterator(diff.hunk_content(file_i, hunk_i))
                    .nth(line_i)
                    .map(|(range, _)| range)
                    .unwrap_or_default();
                rows.push(Item {
                    id: hunk_hash,
                    depth: depth + 2,
                    unselectable: !visit_context_lines && matches!(first.kind, LineKind::Context),
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
    items
}

/// Colored text as one row per line, keeping the colors it arrived with. For
/// output that offers no structure to navigate — a grep, a blame, a plain
/// `diff -u`, anything that isn't a git patch — this is all there is to show.
pub(crate) fn plain_rows(text: &str) -> Vec<Item> {
    rows_of(crate::diff_renderer::parse_ansi_lines(text).lines.iter())
}

fn rows_of<'a>(lines: impl Iterator<Item = &'a crate::diff_renderer::ParsedLine>) -> Vec<Item> {
    lines
        .map(|line| Item {
            depth: 0,
            rendered: Some(Rc::new(rendered_spans(line))),
            ..Default::default()
        })
        .collect()
}

/// The renderer's styled runs for a line, as owned `(text, style)` spans with
/// tabs expanded (matching the built-in hunk-line rendering).
fn rendered_spans(line: &crate::diff_renderer::ParsedLine) -> RenderedRow {
    line.runs
        .iter()
        .map(|(range, style)| {
            let text = line.text[range.clone()].replace('\t', "    ");
            let text = if let Some(link) = line
                .hyperlinks
                .iter()
                .find(|link| link.range.contains(&range.start))
            {
                crate::ui::osc8_hyperlink(&text, &link.uri)
            } else {
                text
            };
            (text, *style)
        })
        .collect()
}

fn create_builtin_diff_items(
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
    visit_context_lines: bool,
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
                    create_hunk_items(
                        Rc::clone(diff),
                        file_i,
                        hunk_i,
                        depth + 1,
                        visit_context_lines,
                    )
                },
            ))
        })
}

fn create_hunk_items(
    diff: Rc<Diff>,
    file_i: usize,
    hunk_i: usize,
    depth: usize,
    visit_context_lines: bool,
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
        visit_context_lines,
    ))
}

fn format_diff_hunk_items(
    diff: Rc<Diff>,
    file_i: usize,
    hunk_i: usize,
    depth: usize,
    hunk_hash: u64,
    visit_context_lines: bool,
) -> Vec<Item> {
    let hunk_content = diff.hunk_content(file_i, hunk_i);

    highlight::line_range_iterator(hunk_content)
        .enumerate()
        .map(|(line_index, (line_range, line))| Item {
            id: hunk_hash,
            unselectable: !visit_context_lines && line.starts_with(' '),
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
    params: &RenderParams,
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

    let blocks = rendered_commits(config, repo, params, &args)?;
    Some(log_items(repo, &blocks))
}

/// Log items from rendered rows that gitu did not run the command for: as git's
/// pager it is handed the renderer's output, and the commit records in it say
/// where each commit begins just as well.
pub(crate) fn rendered_log_items(repo: &Repository, rendered: &str) -> Option<Vec<Item>> {
    Some(log_items(repo, &commits_in(rendered)?))
}

/// Refs a commit carries only decorate it, so failing to list them costs the
/// decorations rather than the log.
fn log_items(repo: &Repository, blocks: &[RenderedCommit]) -> Vec<Item> {
    let references = commit_refs(repo).unwrap_or_default();
    blocks
        .iter()
        .flat_map(|block| commit_block_items(repo, &references, block))
        .collect()
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
    params: &RenderParams,
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
    let output = crate::diff_renderer::run(&command, None, params, Some(dir))?;
    commits_in(&output).or_else(|| {
        log::warn!(
            "log command emitted no commit records; is the '{{commit}}' token in its format?"
        );
        None
    })
}

/// Group rendered rows per commit, as the commit records in them say. `None`
/// when there are no such records, which is how "these rows are not a log" is
/// said.
fn commits_in(rendered: &str) -> Option<Vec<RenderedCommit>> {
    let parsed = crate::diff_renderer::parse_ansi_lines(rendered);
    let blocks = crate::diff_renderer::commit_blocks(&parsed.lines);

    if blocks.iter().all(|block| block.commit.is_none()) {
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
    params: &RenderParams,
    revs: &[String],
) -> HashMap<String, Vec<Rc<RenderedRow>>> {
    if !config.general.log_renderer.enabled || revs.is_empty() {
        return HashMap::new();
    }

    rendered_commits(config, repo, params, revs)
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
    // A renderer states the commit as git printed it, which is abbreviated in
    // most log formats, so it is resolved rather than parsed.
    let commit = block
        .oid
        .as_ref()
        .and_then(|oid| repo.revparse_single(oid).ok())
        .and_then(|object| object.peel_to_commit().ok());

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
/// one that isn't blank or a divider, built by `anchor`): rows before it are
/// separators at the same depth, so a rule the renderer drew above the commit
/// keeps separating commits rather than becoming one's cursor line. Rows after
/// it nest one deeper, so folding the anchor hides them.
fn block_items(
    id: ItemId,
    rows: &[Rc<RenderedRow>],
    anchor: impl Fn(&Rc<RenderedRow>) -> Item,
) -> Vec<Item> {
    let Some(anchor_i) = rows.iter().position(|row| !is_decoration(row)) else {
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

/// Whether a row has nothing to put a cursor on: blank, or a rule the renderer
/// drew between commits (delta's `ol`/`ul`/`box` commit decorations).
fn is_decoration(row: &RenderedRow) -> bool {
    row.iter()
        .flat_map(|(text, _)| text.chars())
        .all(|c| c.is_whitespace() || BOX_DRAWING.contains(&c))
}

const BOX_DRAWING: std::ops::RangeInclusive<char> = '\u{2500}'..='\u{257f}';

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

    // `pick` is what a rebase does with a commit anyway, so only the departures
    // from that are named — in a column, so the commits still line up.
    let styles = &config.style.rebase_todo;
    let style = Style::from(match action {
        TodoAction::Pick => return blank_prefixed(row),
        TodoAction::Reword => &styles.reword,
        TodoAction::Edit => &styles.edit,
        TodoAction::Squash => &styles.squash,
        TodoAction::Fixup => &styles.fixup,
        TodoAction::Drop => &styles.drop,
    });

    iter::once((format!("{:<7}", action.keyword()), style))
        .chain(row.iter().cloned())
        .collect()
}

fn blank_prefixed(row: &Rc<RenderedRow>) -> RenderedRow {
    iter::once((" ".repeat(7), Style::new()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_renderer::parse_ansi_lines;
    use crate::git::diff::{Diff, DiffType};

    const DIFF: &str = "diff --git a/f.rs b/f.rs\n\
                        index 1..2 100644\n\
                        --- a/f.rs\n\
                        +++ b/f.rs\n\
                        @@ -1,2 +1,2 @@\n\
                        \x20keep\n\
                        +added\n";

    fn diff_from(text: &str) -> Rc<Diff> {
        Rc::new(Diff {
            text: text.to_string(),
            diff_type: DiffType::WorkdirToIndex,
            file_diffs: crate::gitu_diff::Parser::new(text).parse_diff().unwrap(),
            commit: None,
        })
    }

    fn hunk_line_rows(items: &[Item]) -> Vec<(usize, bool)> {
        items
            .iter()
            .skip_while(|item| !matches!(item.data, ItemData::HunkLine { .. }))
            .map(|item| (item.depth, item.unselectable))
            .collect()
    }

    #[test]
    fn rendered_spans_keep_hyperlinks() {
        let line = &parse_ansi_lines(
            "plain \x1b]8;;file:///tmp/a.rs:12\x1b\\linked\x1b]8;;\x1b\\ plain\n",
        )
        .lines[0];

        assert_eq!(
            rendered_spans(line)[1].0,
            "\x1b]8;;file:///tmp/a.rs:12\x1b\\linked\x1b]8;;\x1b\\"
        );
    }

    /// A renderer that wraps a long line emits several rows for it, re-emitting
    /// the same record on each (OSC-1717 §6.3). Those continuation rows are one
    /// diff line, so only the first takes the cursor; the rest nest under it, as
    /// a hunk's extra header rows do.
    #[test]
    fn wrapped_row_continuations_nest_under_their_line() {
        let diff = diff_from(DIFF);
        let lines = parse_ansi_lines(
            "\x1b]1717;1\x1b\\\n\
             \x1b]1717;1;a;2;;f.rs\x1b\\+added the first part of a long line\n\
             \x1b]1717;1;a;2;;f.rs\x1b\\ and its wrapped remainder\n",
        )
        .lines;

        let items = rendered_diff_items(&diff, &lines, 0, false, None, false);

        assert_eq!(
            hunk_line_rows(&items),
            vec![(2, false), (3, true)],
            "the wrapped remainder is not a second selectable diff line"
        );
    }

    /// The handshake says a renderer speaks the protocol, not that it said
    /// anything with it. One that greets and then annotates no row at all would
    /// leave a diff of headers with no content, which is worse than the built-in
    /// rendering it displaced — so the built-in rendering stands.
    #[test]
    fn a_render_carrying_no_content_records_falls_back_to_the_built_in_one() {
        let diff = diff_from(DIFF);
        let mut config = crate::config::init_test_config().unwrap();
        config.general.diff_renderer.enabled = true;
        config.general.diff_renderer.command = ["sh", "-c", r"printf '\033]1717;1\033\\\n'"]
            .map(String::from)
            .to_vec();

        let items = create_diff_items(&config, &Default::default(), &diff, 0, false, None);

        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(item.data, ItemData::HunkLine { .. }))
                .count(),
            2,
            "both content lines of the diff are still there"
        );
    }

    /// However many rows a line wraps to, they are still one line: each is a
    /// continuation of the line, not of the row above it.
    #[test]
    fn a_line_wrapping_to_several_rows_stays_one_line() {
        let diff = diff_from(DIFF);
        let row = "\x1b]1717;1;a;2;;f.rs\x1b\\part\n";
        let lines = parse_ansi_lines(&format!("\x1b]1717;1\x1b\\\n{}", row.repeat(3))).lines;

        let items = rendered_diff_items(&diff, &lines, 0, false, None, false);

        assert_eq!(
            hunk_line_rows(&items),
            vec![(2, false), (3, true), (3, true)]
        );
    }

    /// Distinct lines that happen to be adjacent are not continuations.
    #[test]
    fn consecutive_rows_for_different_lines_are_both_selectable() {
        let diff = diff_from(DIFF);
        let lines = parse_ansi_lines(
            "\x1b]1717;1\x1b\\\n\
             \x1b]1717;1;a;2;;f.rs\x1b\\+added\n\
             \x1b]1717;1;c;1;;f.rs\x1b\\ keep\n",
        )
        .lines;

        let items = rendered_diff_items(&diff, &lines, 0, false, None, false);

        // The context row is unselectable for its own reason, but sits at the
        // same depth: it is a diff line, not a continuation.
        assert_eq!(hunk_line_rows(&items), vec![(2, false), (2, true)]);
    }
}
