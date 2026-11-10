use super::{Action, OpTrait};
use crate::{
    app::{App, State},
    item_data::ItemData,
    rebase_todo::TodoAction,
    term::Term,
};
use std::rc::Rc;

/// Move the selected entry `offset` places, keeping the cursor on it.
pub(crate) struct MoveEntry(pub isize);
impl OpTrait for MoveEntry {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        let ItemData::RebaseTodo { todo, index } = target else {
            return None;
        };
        let (todo, index, offset) = (Rc::clone(todo), *index, self.0);

        Some(Rc::new(move |app: &mut App, _term: &mut Term| {
            let moved_to = todo.borrow_mut().move_entry(index, offset);
            app.screen_mut().refresh()?;
            // With the entries either side in view, it shows whether to move
            // it further.
            app.screen_mut()
                .select_matching_with_neighbours_in_view(is_entry(moved_to));
            Ok(())
        }))
    }

    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, _state: &State) -> String {
        if self.0 < 0 { "Move up" } else { "Move down" }.into()
    }
}

pub(crate) struct SetAction(pub TodoAction);
impl OpTrait for SetAction {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        let ItemData::RebaseTodo { todo, index } = target else {
            return None;
        };
        let (todo, index, action) = (Rc::clone(todo), *index, self.0);

        Some(Rc::new(move |app: &mut App, _term: &mut Term| {
            todo.borrow_mut().set_action(index, action);
            app.screen_mut().refresh()?;
            // Marking works down the list, so the cursor moves on to the next
            // entry, staying put on the last one. The entry after it stays in
            // view, to show whether there is another to mark.
            let screen = app.screen_mut();
            if !screen.select_matching_with_neighbours_in_view(is_entry(index + 1)) {
                screen.select_matching_with_neighbours_in_view(is_entry(index));
            }
            Ok(())
        }))
    }

    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, _state: &State) -> String {
        self.0.keyword().into()
    }
}

/// Run the rebase with the list as edited.
pub(crate) struct Start;
impl OpTrait for Start {
    fn get_action(&self, target: &ItemData) -> Option<Action> {
        let ItemData::RebaseTodo { todo, .. } = target else {
            return None;
        };
        let todo = Rc::clone(todo);

        Some(Rc::new(move |app: &mut App, term: &mut Term| {
            let Some(cmd) = todo.borrow().start()? else {
                // Editing git's own list: it is written, and git takes it from
                // here as soon as we exit.
                app.state.exit_code = 0;
                app.state.quit = true;
                return Ok(());
            };

            app.state.screens.pop();
            let result = app.run_cmd_interactive(term, cmd);
            todo.borrow().discard_file();
            result
        }))
    }

    fn is_target_op(&self) -> bool {
        true
    }

    fn display(&self, _state: &State) -> String {
        "Start rebase".into()
    }
}

fn is_entry(index: usize) -> impl Fn(&ItemData) -> bool {
    move |data| matches!(data, ItemData::RebaseTodo { index: at, .. } if *at == index)
}
