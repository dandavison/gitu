use super::{Action, OpTrait};
use crate::{
    Res,
    app::{App, PromptParams, State},
    error::Error,
    item_data::ItemData,
    screen::SearchDirection,
    term::Term,
};
use std::rc::Rc;

pub(crate) struct Search;
impl OpTrait for Search {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, term| {
            prompt_search(app, term, SearchDirection::Forward)
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Search".into()
    }
}

pub(crate) struct SearchBackward;
impl OpTrait for SearchBackward {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, term| {
            prompt_search(app, term, SearchDirection::Backward)
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Search backward".into()
    }
}

pub(crate) struct SearchNext;
impl OpTrait for SearchNext {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| app.screen_mut().search_repeat(false)))
    }

    fn display(&self, _state: &State) -> String {
        "Next match".into()
    }
}

pub(crate) struct SearchPrevious;
impl OpTrait for SearchPrevious {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app, _term| app.screen_mut().search_repeat(true)))
    }

    fn display(&self, _state: &State) -> String {
        "Previous match".into()
    }
}

fn prompt_search(app: &mut App, term: &mut Term, direction: SearchDirection) -> Res<()> {
    // As in less and vim.
    let prompt = match direction {
        SearchDirection::Forward => "/",
        SearchDirection::Backward => "?",
    };

    // As in neovim, the cursor goes to the first match as the query is typed,
    // each time from where the search began, and back there if it is called
    // off.
    let start = app.screen().place();
    let from_start = start.clone();
    let answer = app.prompt(
        term,
        &PromptParams {
            prompt,
            bare: true,
            history: Some(crate::prompt::HistoryKind::Search),
            on_edit: Some(Box::new(move |app, query| {
                let screen = app.screen_mut();
                screen.go_back_to(from_start.clone());
                if query.is_empty() {
                    screen.clear_search();
                } else {
                    // A query part-typed may not match yet, or not be a regex
                    // yet; enter is when that is said.
                    let _ = screen.search(query, direction);
                }
            })),
            ..Default::default()
        },
    );
    app.screen_mut().go_back_to(start);

    let query = match answer {
        Ok(query) if query.is_empty() => {
            app.screen_mut().clear_search();
            return Ok(());
        }
        Ok(query) => query,
        Err(Error::PromptAborted) => {
            app.screen_mut().clear_search();
            return Ok(());
        }
        Err(err) => return Err(err),
    };

    app.screen_mut().search(&query, direction)
}
