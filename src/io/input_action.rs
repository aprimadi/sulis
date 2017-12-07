use state::GameState;
use ui::WidgetState;

#[derive(Debug, Deserialize, Copy, Clone)]
pub enum InputAction {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    MoveCursorUp,
    MoveCursorDown,
    MoveCursorLeft,
    MoveCursorRight,
    MoveToCursor,
}

impl InputAction {
    pub fn fire_action(action: InputAction, state: &mut GameState, root: &mut WidgetState) {
        // TODO figure out a better way to do this than a huge match statement
        // unable to assign a function or closure to an enum, and trait object approach
        // proved unwieldy at best
        use self::InputAction::*;

        match action {
            MoveUp => state.pc_move_by(0, -1),
            MoveDown => state.pc_move_by(0, 1),
            MoveRight => state.pc_move_by(1, 0),
            MoveLeft => state.pc_move_by(-1, 0),
            MoveCursorUp => state.cursor.move_by(0, -1),
            MoveCursorDown => state.cursor.move_by(0, 1),
            MoveCursorLeft => state.cursor.move_by(-1, 0),
            MoveCursorRight => state.cursor.move_by(1, 0),
            MoveToCursor => state.cursor_click(root),
        };
    }
}
