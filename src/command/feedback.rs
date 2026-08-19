//! `/feedback` — rate the last assistant message (#193 Phase C).
//!
//! The one input the learning subsystems cannot synthesise. They can observe
//! what happened and infer whether it worked; they cannot observe whether the
//! person reading it thought the answer was any good.
//!
//! The rating goes to a sidecar relation, never into the message log, so it
//! never reaches model context. A model that could see its last answer was
//! rated badly would start writing for the rating.

use archon_session::feedback::Rating;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

const USAGE: &str = "Usage: /feedback good|bad|clear [note]";

pub(crate) struct FeedbackHandler;

impl CommandHandler for FeedbackHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let Some(snapshot) = ctx.feedback_snapshot.clone() else {
            ctx.emit(TuiEvent::Error(
                "Feedback is unavailable: no session store is attached.".to_string(),
            ));
            return Ok(());
        };

        let verb = args.first().map(String::as_str).unwrap_or("").trim();
        let note = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();

        // No verb shows what is already recorded rather than guessing at one.
        if verb.is_empty() {
            ctx.emit(TuiEvent::TextDelta(describe(&snapshot)));
            return Ok(());
        }

        let Some(message_id) = snapshot.message_id.clone() else {
            ctx.emit(TuiEvent::Error(
                "There is no assistant message to rate yet.".to_string(),
            ));
            return Ok(());
        };

        match verb {
            "good" | "+" | "up" => {
                ctx.pending_effect = Some(CommandEffect::RateMessage {
                    message_id,
                    rating: Some(Rating::Positive.as_str().to_string()),
                    note: note.clone(),
                    expected_version: snapshot.version.clone(),
                })
            }
            "bad" | "-" | "down" => {
                ctx.pending_effect = Some(CommandEffect::RateMessage {
                    message_id,
                    rating: Some(Rating::Negative.as_str().to_string()),
                    note: note.clone(),
                    expected_version: snapshot.version.clone(),
                })
            }
            "clear" | "none" => {
                ctx.pending_effect = Some(CommandEffect::RateMessage {
                    message_id,
                    rating: None,
                    note: String::new(),
                    expected_version: snapshot.version.clone(),
                })
            }
            other => {
                ctx.emit(TuiEvent::Error(format!(
                    "Unknown feedback verb: {other}. {USAGE}"
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "Rate the last assistant message for the learning layer"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["rate"]
    }
}

/// What the handler needs, resolved at the dispatch site.
///
/// The store is sync but the message list is read behind the same lock the
/// session loop holds, so this follows the snapshot pattern the other handlers
/// established rather than reaching for it here.
#[derive(Debug, Clone, Default)]
pub(crate) struct FeedbackSnapshot {
    /// The most recent assistant message, or `None` in a session with none yet.
    pub(crate) message_id: Option<String>,
    /// The rating already on it, if any.
    pub(crate) rating: Option<String>,
    pub(crate) note: Option<String>,
    /// The compare-and-set token to present when changing it.
    pub(crate) version: Option<String>,
}

/// The reply to a bare `/feedback`.
fn describe(snapshot: &FeedbackSnapshot) -> String {
    match (&snapshot.message_id, &snapshot.rating) {
        (None, _) => "\nNo assistant message to rate yet.\n".to_string(),
        (Some(id), None) => {
            format!("\nMessage {id} is unrated. {USAGE}\n")
        }
        (Some(id), Some(rating)) => {
            let note = snapshot
                .note
                .as_deref()
                .map(|note| format!(" — {note}"))
                .unwrap_or_default();
            format!("\nMessage {id} is rated {rating}{note}. {USAGE}\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_with_no_assistant_message_says_so() {
        assert!(describe(&FeedbackSnapshot::default()).contains("No assistant message"));
    }

    #[test]
    fn an_unrated_message_is_reported_as_unrated_with_the_usage() {
        let snapshot = FeedbackSnapshot {
            message_id: Some("7".into()),
            ..FeedbackSnapshot::default()
        };
        let text = describe(&snapshot);
        assert!(text.contains("unrated"), "{text}");
        assert!(text.contains("good|bad|clear"), "{text}");
    }

    /// The note is worth more than the thumb, so it has to be shown back.
    #[test]
    fn an_existing_rating_is_reported_with_its_note() {
        let snapshot = FeedbackSnapshot {
            message_id: Some("7".into()),
            rating: Some("negative".into()),
            note: Some("missed the point".into()),
            version: Some("v1".into()),
        };
        let text = describe(&snapshot);
        assert!(text.contains("negative"), "{text}");
        assert!(text.contains("missed the point"), "{text}");
    }
}
