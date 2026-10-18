use super::{Screen, streamed_items};
use crate::items::RenderParams;
use crate::{
    Res,
    config::Config,
    error::Error,
    git::{
        self,
        diff::{Diff, DiffType},
        status::BranchStatus,
    },
    item_data::{ItemData, SectionHeader},
    items::{self, Item, hash},
    stream::{Input, Snapshot, Stream, Streams},
};
use git2::Repository;
use std::{hash::Hash, path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone, Copy)]
enum SectionID {
    RebaseStatus,
    MergeStatus,
    RevertStatus,
    CherryPickStatus,
    Untracked,
    Stashes,
    RecentCommits,
    BranchStatus,
    UnstagedChanges,
    StagedChanges,
}

impl Hash for SectionID {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let id = match self {
            SectionID::RebaseStatus => "rebase_status",
            SectionID::MergeStatus => "merge_status",
            SectionID::RevertStatus => "revert_status",
            SectionID::CherryPickStatus => "cherry_pick_status",
            SectionID::Untracked => "untracked",
            SectionID::Stashes => "stashes",
            SectionID::RecentCommits => "recent_commits",
            SectionID::BranchStatus => "branch_status",
            SectionID::UnstagedChanges => "unstaged_changes",
            SectionID::StagedChanges => "staged_changes",
        };

        id.hash(state)
    }
}

pub(crate) fn create(
    config: Arc<Config>,
    repo: Rc<Repository>,
    params: RenderParams,
) -> Res<Screen> {
    let mut screen = Screen::streamed(
        Arc::clone(&config),
        params,
        Box::new(move |params: RenderParams| {
            let status = git::status(repo.workdir().ok_or(Error::NoRepoWorkdir)?)?;
            let untracked_files = status
                .files
                .iter()
                .filter(|status| status.is_untracked())
                .map(|status| &status.path)
                .collect::<Vec<_>>();

            let untracked = untracked_list(&untracked_files);

            let head = if let Some(rebase) = git::rebase_status(&repo)? {
                vec![Item {
                    id: hash(SectionID::RebaseStatus),
                    data: ItemData::Header(SectionHeader::Rebase(rebase.head_name, rebase.onto)),
                    ..Default::default()
                }]
                .into_iter()
            } else if let Some(merge) = git::merge_status(&repo)? {
                vec![Item {
                    id: hash(SectionID::MergeStatus),
                    data: ItemData::Header(SectionHeader::Merge(merge.head)),
                    ..Default::default()
                }]
                .into_iter()
            } else if let Some(revert) = git::revert_status(&repo)? {
                vec![Item {
                    id: hash(SectionID::RevertStatus),
                    data: ItemData::Header(SectionHeader::Revert(revert.head)),
                    ..Default::default()
                }]
                .into_iter()
            } else if let Some(cherry_pick) = git::cherry_pick_status(&repo)? {
                vec![Item {
                    id: hash(SectionID::CherryPickStatus),
                    data: ItemData::Header(SectionHeader::CherryPick(cherry_pick.head)),
                    ..Default::default()
                }]
                .into_iter()
            } else {
                branch_status_items(&status.branch_status, &config)?.into_iter()
            }
            .chain(if untracked.is_empty() {
                vec![]
            } else {
                vec![
                    items::blank_line(),
                    Item {
                        id: hash(SectionID::Untracked),
                        depth: 0,
                        data: ItemData::AllUntracked(
                            untracked_files.iter().map(PathBuf::from).collect(),
                        ),
                        ..Default::default()
                    },
                ]
            })
            .chain(untracked)
            .collect::<Vec<_>>();

            let context = params.context.as_deref();
            let sections = [
                (
                    SectionID::UnstagedChanges,
                    git::diff_unstaged(repo.as_ref(), context)?,
                ),
                (
                    SectionID::StagedChanges,
                    git::diff_staged(repo.as_ref(), context)?,
                ),
            ]
            .into_iter()
            .filter(|(_, diff)| !diff.file_diffs.is_empty())
            .map(|(section, diff)| (section, Rc::new(diff)))
            .collect::<Vec<_>>();

            let tail =
                create_stash_list_section_items(repo.as_ref(), config.general.stash_list_limit)
                    .chain(create_log_section_items(
                        repo.as_ref(),
                        config.general.recent_commits_limit,
                    ))
                    .collect::<Vec<_>>();

            let streams = Streams(
                sections
                    .iter()
                    .map(|(_, diff)| rendering(&config, &params, diff))
                    .collect::<Res<_>>()?,
            );
            let (config, built_params) = (Arc::clone(&config), params.clone());
            streamed_items(streams, &params, move |snapshots| {
                Ok(status_items(
                    &config,
                    &built_params,
                    &head,
                    &sections,
                    snapshots,
                    &tail,
                ))
            })
        }),
    )?;

    screen.is_status = true;
    // Unstaged files come before staged ones, and are the only other files here.
    if screen.config.general.status_cursor_on_changes
        && let Some(file) = screen.find_item(|item| matches!(item.data, ItemData::Delta { .. }))
    {
        screen.cursor = file;
    }
    Ok(screen)
}

fn untracked_list(files: &[&String]) -> Vec<Item> {
    files
        .iter()
        .map(|path| Item {
            id: hash(path),
            depth: 1,
            data: ItemData::Untracked(PathBuf::from(path)),
            ..Default::default()
        })
        .collect::<Vec<_>>()
}

fn branch_status_items(status: &BranchStatus, config: &Config) -> Res<Vec<Item>> {
    let Some(ref head) = status.local else {
        return Ok(vec![Item {
            id: hash(SectionID::BranchStatus),
            depth: 0,
            data: ItemData::Header(SectionHeader::NoBranch),
            ..Default::default()
        }]);
    };

    let mut items = vec![Item {
        id: hash(SectionID::BranchStatus),
        depth: 0,
        data: ItemData::Header(SectionHeader::OnBranch(head.clone())),
        ..Default::default()
    }];

    if config.general.verbose_branch_status
        && let Some(ref upstream_name) = status.remote
    {
        items.push(Item {
            id: hash(SectionID::BranchStatus),
            depth: 1,
            unselectable: true,
            data: ItemData::BranchStatus(upstream_name.clone(), status.ahead, status.behind),
            ..Default::default()
        });
    }

    Ok(items)
}

/// `diff` drawn by the renderer as it is drawn.
fn rendering(config: &Config, params: &RenderParams, diff: &Diff) -> Res<Stream> {
    let renderer =
        crate::diff_renderer::renderer_command(&config.general.diff_renderer.command, params, None);
    Stream::start(Input::Text(diff.text.clone()), renderer, u64::MAX).map_err(Error::GitDiff)
}

/// The status items, with each section's diff as the renderer has drawn it
/// so far. A section is shown only once those above it are drawn whole.
fn status_items(
    config: &Config,
    params: &RenderParams,
    head: &[Item],
    sections: &[(SectionID, Rc<Diff>)],
    snapshots: Vec<Snapshot>,
    tail: &[Item],
) -> Vec<Item> {
    let mut items = head.to_vec();
    for ((section, diff), snapshot) in sections.iter().zip(snapshots) {
        items.extend(create_status_section_items(
            config, params, *section, diff, &snapshot,
        ));
        if !snapshot.done {
            return items;
        }
    }
    items.extend_from_slice(tail);
    items
}

fn create_status_section_items(
    config: &Config,
    params: &RenderParams,
    section: SectionID,
    diff: &Rc<Diff>,
    snapshot: &Snapshot,
) -> Vec<Item> {
    let item_data = match section {
        SectionID::UnstagedChanges => ItemData::AllUnstaged(diff.file_diffs.len()),
        SectionID::StagedChanges => ItemData::AllStaged(diff.file_diffs.len()),
        _ => unreachable!("no other status section should be created"),
    };
    let drawn = items::drawn_diff_items(
        config,
        diff,
        snapshot.rendered.as_deref(),
        snapshot.done,
        1,
        true,
        None,
    );

    [
        items::blank_line(),
        Item {
            id: hash(section),
            depth: 0,
            data: item_data,
            ..Default::default()
        },
    ]
    .into_iter()
    .chain(
        items::headed_as_configured(config, params, drawn)
            .into_iter()
            .map(folded_as_in_other_section),
    )
    .collect()
}

/// A file first staged or unstaged is folded as it was in the section it came
/// from.
fn folded_as_in_other_section(mut item: Item) -> Item {
    if let ItemData::Delta { diff, file_i, .. } = &item.data {
        let other = match diff.diff_type {
            DiffType::WorkdirToIndex => DiffType::IndexToTree,
            DiffType::IndexToTree => DiffType::WorkdirToIndex,
            DiffType::TreeToTree => unreachable!("status diffs are staged or unstaged"),
        };
        item.folds_like = Some(items::file_id_as(diff, *file_i, &other));
    }
    item
}

fn create_stash_list_section_items<'a>(
    repo: &Repository,
    limit: usize,
) -> impl Iterator<Item = Item> + 'a {
    let stashes = items::stash_list(repo, limit).unwrap();
    if stashes.is_empty() {
        vec![]
    } else {
        vec![
            items::blank_line(),
            Item {
                id: hash(SectionID::Stashes),
                depth: 0,
                data: ItemData::Header(SectionHeader::Stashes),
                ..Default::default()
            },
        ]
    }
    .into_iter()
    .chain(stashes)
}

fn create_log_section_items<'a>(
    repo: &Repository,
    limit: usize,
) -> impl Iterator<Item = Item> + 'a {
    let (header, log) = if limit == 0 {
        (vec![], vec![])
    } else {
        (
            vec![
                Item {
                    depth: 0,
                    unselectable: true,
                    ..Default::default()
                },
                Item {
                    id: hash(SectionID::RecentCommits),
                    depth: 0,
                    data: ItemData::Header(SectionHeader::RecentCommits),
                    ..Default::default()
                },
            ],
            items::log(repo, limit, None, None).unwrap(),
        )
    };

    header.into_iter().chain(log)
}
