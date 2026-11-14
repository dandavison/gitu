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
            let (rendered, failed) =
                match rendered_blame(&config, &params, &repo, &file_path, commit.as_deref()) {
                    Ok(rendered) => (rendered, None),
                    Err(failed) => (None, Some(failed)),
                };
            let rendered = rendered.filter(|rows| rows.len() == blame_lines.len());
            // A renderer that failed leaves gitu's own drawing, under what went
            // wrong.
            let failed = failed.map(|failed| crate::items::error_row(&config, failed));

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
                return Ok(failed.into_iter().chain([header]).chain(lines).collect());
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

            Ok(failed.into_iter().chain([header]).chain(entries).collect())
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

/// `GIT_CONFIG_PARAMETERS` for delta drawing the blame of `file_path`, after
/// those `given`: no wrapping, so that each line of the file is one row, and the
/// language of the file, which delta would otherwise take from the git blame
/// command that called it.
fn delta_config_parameters(file_path: &str, given: Option<&str>) -> String {
    let language = std::path::Path::new(file_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(" 'delta.default-language={extension}'"))
        .unwrap_or_default();
    let ours = format!("'delta.wrap-max-lines=0'{language}");
    match given {
        Some(given) => format!("{given} {ours}"),
        None => ours,
    }
}

/// The rows the renderer draws for git blame's output, one per line of the file:
/// `None` if it draws none, and what to tell the user if it fails.
fn rendered_blame(
    config: &Config,
    params: &RenderParams,
    repo: &Repository,
    file_path: &str,
    commit: Option<&str>,
) -> Result<Option<Vec<Rc<crate::items::RenderedRow>>>, String> {
    let Some(text) = git::blame_text(repo, file_path, commit) else {
        return Ok(None);
    };
    let given = std::env::var("GIT_CONFIG_PARAMETERS").ok();
    let parameters = delta_config_parameters(file_path, given.as_deref());
    let output = crate::diff_renderer::run_with_env(
        &config.general.diff_renderer.command,
        Some(&text),
        params,
        repo.workdir(),
        &[("GIT_CONFIG_PARAMETERS", parameters)],
    )?;
    Ok(output.and_then(|output| {
        crate::items::plain_rows(&output)
            .into_iter()
            .map(|item| item.rendered)
            .collect()
    }))
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
