use crossterm::event::KeyCode;

use crate::app::App;

pub(crate) fn handle_key(app: &mut App, key: KeyCode) -> bool {
    if app.thinking_archive.is_none() {
        return false;
    }

    match key {
        KeyCode::Up => app.select_previous_thinking_block(),
        KeyCode::Down => app.select_next_thinking_block(),
        KeyCode::Enter => app.expand_selected_thinking_block(),
        KeyCode::Esc => app.close_thinking_archive(),
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_archive_input_is_not_consumed() {
        assert!(!handle_key(&mut App::new(), KeyCode::Esc));
    }
}
