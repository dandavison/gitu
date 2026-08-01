use super::Screen;
use crate::{Res, config::Config, items, items::RenderedRow, menu::Menu, rebase_todo::RebaseTodo};
use git2::Repository;
use ratatui::layout::Size;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

type CommitRows = HashMap<String, Vec<Rc<RenderedRow>>>;

pub(crate) fn create(
    config: Arc<Config>,
    repo: Rc<Repository>,
    size: Size,
    todo: Rc<RefCell<RebaseTodo>>,
) -> Res<Screen> {
    // The commits themselves don't change while the list is edited, so their
    // rendered rows are fetched once per width rather than on every reorder.
    let cache: RefCell<Option<(u16, CommitRows)>> = RefCell::new(None);
    let revs = format!("{}..HEAD", todo.borrow().base.to_string_lossy());
    let screen_config = Arc::clone(&config);

    let mut screen = Screen::new(
        config,
        size,
        Box::new(move |size: Size| {
            let mut cache = cache.borrow_mut();
            if cache.as_ref().is_none_or(|(width, _)| *width != size.width) {
                let width = (size.width as usize).saturating_sub(2);
                let rows = items::rendered_commit_rows(&screen_config, &repo, width, &revs);
                *cache = Some((size.width, rows));
            }

            let (_, rows) = cache.as_ref().expect("just populated");
            Ok(items::rebase_todo_items(&screen_config, &repo, &todo, rows))
        }),
    )?;

    screen.menu = Some(Menu::RebaseTodo);
    Ok(screen)
}
