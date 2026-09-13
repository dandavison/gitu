use super::Screen;
use crate::{
    Res,
    config::Config,
    items::{self, log},
};
use git2::{Oid, Repository};
use regex::Regex;
use std::{rc::Rc, sync::Arc};

pub(crate) fn create(
    config: Arc<Config>,
    repo: Rc<Repository>,
    size: (u16, u16),
    limit: usize,
    rev: Option<Oid>,
    msg_regex: Option<Regex>,
) -> Res<Screen> {
    Screen::new(
        Arc::clone(&config),
        size,
        Box::new(move |size: (u16, u16)| {
            if config.general.log_renderer.enabled
                && let Some(items) = items::rendered_log(
                    &config,
                    &repo,
                    (size.0 as usize).saturating_sub(2),
                    limit,
                    rev,
                    msg_regex.as_ref(),
                )
            {
                return Ok(items);
            }

            log(&repo, limit, rev, msg_regex.clone())
        }),
    )
}
