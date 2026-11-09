use crate::items::RenderParams;
use std::{rc::Rc, sync::Arc};

use crate::{
    Res,
    config::Config,
    git,
    item_data::{ItemData, SectionHeader},
    items::{Item, hash},
};
use git2::Repository;

use super::Screen;

pub(crate) fn create(
    config: Arc<Config>,
    repo: Rc<Repository>,
    params: RenderParams,
    file_path: String,
    commit: Option<String>,
    target_line: Option<u32>,
) -> Res<Screen> {
    let mut screen = Screen::new(
        Arc::clone(&config),
        params,
        Box::new(move |params: RenderParams| {
            let commit_display = commit.as_deref().unwrap_or("HEAD").to_string();
            let blame_lines = git::blame(repo.as_ref(), &file_path, commit.as_deref())?;
            let rendered = rendered_blame(&config, &params, &repo, &file_path, commit.as_deref())
                .filter(|rows| rows.len() == blame_lines.len());

            let header = Item {
                id: hash(("blame_header", file_path.as_str(), commit_display.as_str())),
                depth: 0,
                unselectable: true,
                data: ItemData::Header(SectionHeader::Blame(file_path.clone(), commit_display)),
                ..Default::default()
            };

            // As the renderer draws git blame, its rows in place of gitu's own,
            // with no header row per commit, since its rows show the commit. The
            // first line of each commit's run stands in for the header, as the
            // stop for moving between commits.
            if let Some(rendered) = rendered {
                let mut previous = None::<String>;
                let lines = blame_lines.into_iter().zip(rendered).map(|(line, row)| {
                    let starts_run = previous.as_deref() != Some(line.commit_hash.as_str());
                    previous = Some(line.commit_hash.clone());
                    let data = if starts_run {
                        ItemData::BlameHeader {
                            commit_hash: line.commit_hash.clone(),
                            short_hash: line.short_hash.clone(),
                            _author: line.author.clone(),
                            _author_time: line.author_time,
                            summary: line.summary.clone(),
                            file_path: file_path.clone(),
                            line_num: line.orig_line_num,
                            blamed_line_num: line.line_num,
                        }
                    } else {
                        ItemData::BlameCodeLine {
                            line_num: line.line_num,
                            orig_line_num: line.orig_line_num,
                            content: line.content,
                            commit_hash: line.commit_hash.clone(),
                            file_path: file_path.clone(),
                        }
                    };
                    Item {
                        id: hash(("blame_code", line.commit_hash.as_str(), line.line_num)),
                        depth: 0,
                        rendered: Some(row),
                        data,
                        ..Default::default()
                    }
                });
                return Ok(std::iter::once(header).chain(lines).collect());
            }

            let entries = blame_lines
                .into_iter()
                .scan(None::<String>, |prev_hash, line| {
                    let is_new_chunk = prev_hash.as_deref() != Some(line.commit_hash.as_str());
                    *prev_hash = Some(line.commit_hash.clone());
                    Some((line, is_new_chunk))
                })
                .flat_map(|(line, is_new_chunk)| {
                    let mut items = Vec::new();

                    if is_new_chunk {
                        items.push(Item {
                            id: hash(("blame_chunk", line.commit_hash.as_str(), line.line_num)),
                            depth: 0,
                            data: ItemData::BlameHeader {
                                commit_hash: line.commit_hash.clone(),
                                short_hash: line.short_hash.clone(),
                                _author: line.author.clone(),
                                _author_time: line.author_time,
                                summary: line.summary.clone(),
                                file_path: file_path.clone(),
                                line_num: line.orig_line_num,
                                blamed_line_num: line.line_num,
                            },
                            ..Default::default()
                        });
                    }

                    items.push(Item {
                        id: hash(("blame_code", line.commit_hash.as_str(), line.line_num)),
                        depth: 0,
                        data: ItemData::BlameCodeLine {
                            line_num: line.line_num,
                            orig_line_num: line.orig_line_num,
                            content: line.content,
                            commit_hash: line.commit_hash,
                            file_path: file_path.clone(),
                        },
                        ..Default::default()
                    });

                    items
                });

            Ok(std::iter::once(header).chain(entries).collect())
        }),
    )?;

    if let Some(line) = target_line
        && !screen.select_matching(
            |data| matches!(data, ItemData::BlameCodeLine { line_num, .. } if *line_num == line),
        )
    {
        screen.select_last_matching(|data| {
            matches!(data, ItemData::BlameHeader { blamed_line_num, .. }
                if *blamed_line_num <= line)
        });
    }

    Ok(screen)
}

/// The rows the renderer draws for git blame's output, one per line of the file,
/// or `None` if it draws none. delta is told not to wrap, so that a long line
/// stays one row.
fn rendered_blame(
    config: &Config,
    params: &RenderParams,
    repo: &Repository,
    file_path: &str,
    commit: Option<&str>,
) -> Option<Vec<Rc<crate::items::RenderedRow>>> {
    let text = git::blame_text(repo, file_path, commit)?;
    let no_wrap = "'delta.wrap-max-lines=0'";
    let parameters = match std::env::var("GIT_CONFIG_PARAMETERS") {
        Ok(given) => format!("{given} {no_wrap}"),
        Err(_) => no_wrap.to_owned(),
    };
    let output = crate::diff_renderer::run_with_env(
        &config.general.diff_renderer.command,
        Some(&text),
        params,
        repo.workdir(),
        &[("GIT_CONFIG_PARAMETERS", parameters)],
    )?;
    crate::items::plain_rows(&output)
        .into_iter()
        .map(|item| item.rendered)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// delta cannot tell the language of git blame's output when gitu, not
    /// `git blame <file>`, runs it, so it is told by the file's extension.
    #[test]
    fn delta_is_told_the_language_of_the_file_blamed() {
        assert_eq!(
            delta_config_parameters("src/handlers/blame.rs", None),
            "'delta.wrap-max-lines=0' 'delta.default-language=rs'"
        );
        assert_eq!(
            delta_config_parameters("Makefile", Some("'a.b=c'")),
            "'a.b=c' 'delta.wrap-max-lines=0'"
        );
    }
}
