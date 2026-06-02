use crossterm::event::KeyCode;

use crate::app::App;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum AskUserKeyOutcome {
    Handled,
    Submit(String),
    Cancel,
}

pub(super) fn handle_ask_user_key(app: &mut App, code: &KeyCode) -> Option<AskUserKeyOutcome> {
    app.ask_user_prompt.as_ref()?;
    match code {
        KeyCode::Enter => {
            let answer = std::mem::take(&mut app.ask_user_draft);
            app.ask_user_prompt = None;
            Some(AskUserKeyOutcome::Submit(answer))
        }
        KeyCode::Esc => {
            app.ask_user_draft.clear();
            app.ask_user_prompt = None;
            Some(AskUserKeyOutcome::Cancel)
        }
        KeyCode::Backspace => {
            app.ask_user_draft.pop();
            Some(AskUserKeyOutcome::Handled)
        }
        KeyCode::Char(ch) => {
            app.ask_user_draft.push(*ch);
            Some(AskUserKeyOutcome::Handled)
        }
        _ => Some(AskUserKeyOutcome::Handled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_returns_draft_and_clears_prompt() {
        let mut app = App::new();
        app.ask_user_prompt = Some("Continue?".into());
        app.ask_user_draft = "yes".into();

        let outcome = handle_ask_user_key(&mut app, &KeyCode::Enter);

        assert_eq!(outcome, Some(AskUserKeyOutcome::Submit("yes".into())));
        assert!(app.ask_user_prompt.is_none());
        assert!(app.ask_user_draft.is_empty());
    }

    #[test]
    fn char_updates_draft_without_chat_input() {
        let mut app = App::new();
        app.ask_user_prompt = Some("Continue?".into());

        let outcome = handle_ask_user_key(&mut app, &KeyCode::Char('y'));

        assert_eq!(outcome, Some(AskUserKeyOutcome::Handled));
        assert_eq!(app.ask_user_draft, "y");
        assert!(app.input.text().is_empty());
    }
}
