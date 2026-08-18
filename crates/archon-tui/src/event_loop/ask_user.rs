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
            app.ask_user_prompt_kind = None;
            Some(AskUserKeyOutcome::Submit(answer))
        }
        KeyCode::Esc => {
            let is_plan_approval = matches!(
                app.ask_user_prompt_kind,
                Some(archon_core::agent::AskUserPromptKind::PlanApproval)
            );
            app.ask_user_draft.clear();
            app.ask_user_prompt = None;
            app.ask_user_prompt_kind = None;
            if is_plan_approval {
                Some(AskUserKeyOutcome::Submit(
                    "reject: cancelled by user".into(),
                ))
            } else {
                Some(AskUserKeyOutcome::Cancel)
            }
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

    #[test]
    fn returns_none_when_no_question_is_active() {
        let mut app = App::new();

        let outcome = handle_ask_user_key(&mut app, &KeyCode::Char('y'));

        assert_eq!(outcome, None);
        assert!(app.ask_user_draft.is_empty());
    }

    #[test]
    fn escape_cancels_and_clears_draft() {
        let mut app = App::new();
        app.ask_user_prompt = Some("Continue?".into());
        app.ask_user_draft = "partial".into();

        let outcome = handle_ask_user_key(&mut app, &KeyCode::Esc);

        assert_eq!(outcome, Some(AskUserKeyOutcome::Cancel));
        assert!(app.ask_user_prompt.is_none());
        assert!(app.ask_user_draft.is_empty());
    }

    #[test]
    fn plan_approval_escape_submits_cancelled_rejection() {
        let mut app = App::new();
        app.ask_user_prompt =
            Some("Plan approval: approve / approve-edits / edit / reject: <reason>".into());
        app.ask_user_prompt_kind = Some(archon_core::agent::AskUserPromptKind::PlanApproval);

        let outcome = handle_ask_user_key(&mut app, &KeyCode::Esc);

        assert_eq!(
            outcome,
            Some(AskUserKeyOutcome::Submit(
                "reject: cancelled by user".into()
            ))
        );
        assert!(app.ask_user_prompt_kind.is_none());
    }

    #[test]
    fn ordinary_prompt_with_plan_approval_text_still_cancels() {
        let mut app = App::new();
        app.ask_user_prompt = Some("Plan approval: this is an ordinary question".into());
        app.ask_user_prompt_kind = Some(archon_core::agent::AskUserPromptKind::Ordinary);

        let outcome = handle_ask_user_key(&mut app, &KeyCode::Esc);

        assert_eq!(outcome, Some(AskUserKeyOutcome::Cancel));
    }

    #[test]
    fn backspace_and_non_text_keys_are_handled_locally() {
        let mut app = App::new();
        app.ask_user_prompt = Some("Continue?".into());
        app.ask_user_draft = "yes".into();

        let backspace = handle_ask_user_key(&mut app, &KeyCode::Backspace);
        let ignored = handle_ask_user_key(&mut app, &KeyCode::Left);

        assert_eq!(backspace, Some(AskUserKeyOutcome::Handled));
        assert_eq!(ignored, Some(AskUserKeyOutcome::Handled));
        assert_eq!(app.ask_user_draft, "ye");
        assert_eq!(app.ask_user_prompt.as_deref(), Some("Continue?"));
    }
}
