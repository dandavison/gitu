use crate::Res;
use crate::config::Config;
use crate::error::Error;
use crate::git::diff::Diff;
use crate::gitu_diff::Status;
use crate::highlight;
use crate::item_data::ItemData;
use crate::item_data::Ref;
use crate::item_data::SectionHeader;
use git2::Oid;
use git2::Repository;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use regex::Regex;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::iter;
use std::rc::Rc;
use std::sync::Arc;

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
    /// used verbatim by [`Self::to_line`] in place of computing from `data`.
    pub(crate) rendered: Option<Rc<RenderedRow>>,
}

impl Item {
    pub fn to_line(&'_ self, config: Arc<Config>) -> Line<'_> {
        if let Some(rendered) = &self.rendered {
            return Line::from_iter(
                rendered
                    .iter()
                    .map(|(text, style)| Span::styled(text.clone(), *style)),
            );
        }
        match self.data.clone() {
            ItemData::Raw(content) => Line::raw(content),
            ItemData::AllUnstaged(count) => Line::from(vec![
                Span::styled("Unstaged changes", &config.style.section_header),
                Span::raw(format!(" ({count})")),
            ]),
            ItemData::AllStaged(count) => Line::from(vec![
                Span::styled("Staged changes", &config.style.section_header),
                Span::raw(format!(" ({count})")),
            ]),
            ItemData::AllUntracked(_) => {
                Line::styled("Untracked files", &config.style.section_header)
            }
            ItemData::Reference { kind, prefix } => {
                let (reference, style) = match kind {
                    Ref::Tag(tag) => (tag, &config.style.tag),
                    Ref::Head(branch) => (branch, &config.style.branch),
                    Ref::Remote(remote) => (remote, &config.style.remote),
                };

                Line::from(vec![Span::raw(prefix), Span::styled(reference, style)])
            }
            ItemData::Commit {
                short_id,
                associated_references,
                summary,
                ..
            } => Line::from_iter(itertools::intersperse(
                iter::once(Span::styled(short_id, &config.style.hash))
                    .chain(
                        associated_references
                            .into_iter()
                            .map(|reference| match reference {
                                Ref::Tag(tag) => Span::styled(tag, &config.style.tag),
                                Ref::Head(branch) => Span::styled(branch, &config.style.branch),
                                Ref::Remote(remote) => Span::styled(remote, &config.style.remote),
                            }),
                    )
                    .chain([Span::raw(summary)]),
                Span::raw(" "),
            )),
            ItemData::Untracked(path) => Line::styled(
                path.to_string_lossy().into_owned(),
                &config.style.file_header,
            ),
            ItemData::Delta { diff, file_i, .. } => {
                let file_diff = &diff.file_diffs[file_i];

                let content = format!(
                    "{:8}   {}",
                    format!("{:?}", file_diff.header.status).to_lowercase(),
                    match file_diff.header.status {
                        Status::Renamed | Status::Copied => format!(
                            "{} -> {}",
                            &file_diff.header.old_file.fmt(&diff.text),
                            &file_diff.header.new_file.fmt(&diff.text)
                        ),
                        Status::Deleted => file_diff.header.old_file.fmt(&diff.text).to_string(),
                        Status::Added => file_diff.header.new_file.fmt(&diff.text).to_string(),
                        Status::Modified => file_diff.header.new_file.fmt(&diff.text).to_string(),
                        Status::Unmerged => file_diff.header.new_file.fmt(&diff.text).to_string(),
                    }
                );

                Line::styled(content, &config.style.file_header)
            }
            ItemData::Hunk {
                diff,
                file_i,
                hunk_i,
            } => {
                let file_diff = &diff.file_diffs[file_i];
                let hunk = &file_diff.hunks[hunk_i];
                let content = &diff.text[hunk.header.range.clone()];

                Line::styled(content.to_string(), &config.style.hunk_header)
            }
            ItemData::HunkLine {
                diff,
                file_i,
                hunk_i,
                line_range,
                line_i,
                ..
            } => {
                let hunk_highlights =
                    highlight::highlight_hunk(self.id, &config, &Rc::clone(&diff), file_i, hunk_i);

                let hunk_content = &diff.hunk_content(file_i, hunk_i);
                let hunk_line = &hunk_content[line_range.clone()];

                let line_highlights = hunk_highlights.get_line_highlights(line_i);

                Line::from_iter(line_highlights.iter().map(|(highlight_range, style)| {
                    Span::styled(
                        hunk_line[highlight_range.clone()].replace("\t", "    "),
                        *style,
                    )
                }))
            }
            ItemData::Stash { message, id, .. } => Line::from(vec![
                Span::styled(format!("stash@{id}"), &config.style.hash),
                Span::raw(format!(" {message}")),
            ]),
            ItemData::Header(header) => {
                let content = match header {
                    SectionHeader::Remote(remote) => format!("Remote {remote}"),
                    SectionHeader::Tags => "Tags".to_string(),
                    SectionHeader::Branches => "Branches".to_string(),
                    SectionHeader::NoBranch => "No branch".to_string(),
                    SectionHeader::OnBranch(branch) => branch,
                    SectionHeader::Rebase(head, onto) => format!("Rebasing {head} onto {onto}"),
                    SectionHeader::Merge(head) => format!("Merging {head}"),
                    SectionHeader::Revert(head) => format!("Reverting {head}"),
                    SectionHeader::CherryPick(head) => format!("Cherry-picking {head}"),
                    SectionHeader::Stashes => "Stashes".to_string(),
                    SectionHeader::RecentCommits => "Recent commits".to_string(),
                    SectionHeader::Commit(oid) => format!("commit {oid}"),
                    SectionHeader::StashRef(stash_ref) => stash_ref,
                    SectionHeader::StagedChanges(count) => format!("Staged changes ({count})"),
                    SectionHeader::UnstagedChanges(count) => format!("Unstaged changes ({count})"),
                    SectionHeader::UntrackedFiles(count) => format!("Untracked files ({count})"),
                    SectionHeader::Blame(file, commit) => {
                        format!("Blame {file} @ {commit}")
                    }
                };

                Line::styled(content, &config.style.section_header)
            }
            ItemData::Error(err) => Line::raw(err),
            ItemData::BlameHeader {
                short_hash,
                summary,
                ..
            } => Line::from(vec![
                Span::styled(format!("{:<8}", short_hash), &config.style.hash),
                Span::raw(" "),
                Span::raw(summary.clone()),
            ]),
            ItemData::BlameCodeLine {
                blame_file,
                line_i,
                line_num,
                content,
                ..
            } => {
                let mut spans = vec![Span::styled(
                    format!("{:>4} ", line_num),
                    &config.style.blame.line_num,
                )];

                for (range, style) in blame_file.highlights.get_line_highlights(line_i) {
                    if !range.is_empty() && range.end <= content.len() {
                        spans.push(Span::styled(
                            content[range.clone()].replace('\t', "    "),
                            *style,
                        ));
                    }
                }

                Line::from(spans).style(Style::from(&config.style.blame.code_line))
            }
        }
    }
}

/// Build the items for a diff. When the configured colorizer speaks the OSC-1717
/// protocol, the diff is rendered freely by it (e.g. delta, side-by-side) and its
/// styled rows drive the content lines; otherwise the built-in items are used
/// (styled per-hunk by [`highlight`], including a `--color-only` colorizer).
pub(crate) fn create_diff_items(
    config: &Config,
    width: usize,
    diff: &Rc<Diff>,
    depth: usize,
    default_collapsed: bool,
    commit: Option<String>,
) -> Vec<Item> {
    if config.general.diff_colorizer.enabled
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

/// Render the diff through the OSC-1717 colorizer and lay its rows out under
/// gitu's own file/hunk structure (so collapse, navigation and file/hunk/line
/// staging keep working). Each rendered content row is mapped back to its
/// content-line coordinates via its metadata; decoration rows are dropped.
/// Returns `None` (fall back to built-in) if the colorizer doesn't speak the
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

    let output =
        crate::diff_colorizer::run(&config.general.diff_colorizer.command, &diff.text, width)?;
    let parsed = crate::diff_colorizer::parse_ansi_lines(&output);
    parsed.protocol_version?; // Not an OSC-1717 renderer: fall back to built-in.

    use crate::diff_colorizer::LineKind;

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
            LineKind::FileHeader => {}
            LineKind::HunkHeader => {
                if line.text.trim().is_empty() {
                    continue;
                }
                if let Some(key) = crate::diff_colorizer::resolve_hunk(diff, first) {
                    headers_by_hunk
                        .entry(key)
                        .or_default()
                        .push(rendered_spans(line));
                }
            }
            LineKind::Context | LineKind::Added | LineKind::Deleted => {
                let Some((file_i, hunk_i, line_i)) =
                    crate::diff_colorizer::resolve_line(diff, first)
                else {
                    continue;
                };
                // A fused side-by-side change row also carries its replacement, so
                // stage every record in the same hunk.
                let line_indices = line
                    .records
                    .iter()
                    .filter_map(|meta| crate::diff_colorizer::resolve_line(diff, meta))
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

/// The colorizer's styled runs for a line, as owned `(text, style)` spans with
/// tabs expanded (matching the built-in hunk-line rendering).
fn rendered_spans(line: &crate::diff_colorizer::ParsedLine) -> RenderedRow {
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

    let references: Vec<_> = repo
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
        .collect();

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
