use super::{Action, OpTrait};
use crate::{
    Res,
    app::{App, State},
    item_data::ItemData,
    screen,
    term::Term,
};
use std::{rc::Rc, sync::Arc};

pub(crate) struct Status;
impl OpTrait for Status {
    fn get_action(&self, _target: &ItemData) -> Option<Action> {
        Some(Rc::new(|app: &mut App, _term: &mut Term| {
            goto_status_screen(app)
        }))
    }

    fn display(&self, _state: &State) -> String {
        "Status".into()
    }
}

/// Leaves the screen gitu was started on at the bottom of the stack: it is the
/// status screen already when gitu was started without arguments, and where it
/// isn't (`gitu log`, git's pager, the sequence editor) quitting status should
/// land back on it.
fn goto_status_screen(app: &mut App) -> Res<()> {
    app.state.screens.drain(1..);
    if app.state.screens[0].is_status {
        return Ok(());
    }

    let params = app.render_params(app.state.screens[0].size);
    app.state.screens.push(screen::status::create(
        Arc::clone(&app.state.config),
        Rc::clone(&app.state.repo),
        params,
    )?);

    Ok(())
}
