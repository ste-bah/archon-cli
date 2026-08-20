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

const USAGE: &str = "Usage: /feedback good|bad|clear [note], or /feedback list";

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

        if verb == "list" || verb == "all" {
            ctx.emit(TuiEvent::TextDelta(list(&snapshot)));
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
                    message_digest: snapshot.message_digest.clone(),
                    rating: Some(Rating::Positive.as_str().to_string()),
                    note: note.clone(),
                    expected_version: snapshot.version.clone(),
                })
            }
            "bad" | "-" | "down" => {
                ctx.pending_effect = Some(CommandEffect::RateMessage {
                    message_id,
                    message_digest: snapshot.message_digest.clone(),
                    rating: Some(Rating::Negative.as_str().to_string()),
                    note: note.clone(),
                    expected_version: snapshot.version.clone(),
                })
            }
            "clear" | "none" => {
                ctx.pending_effect = Some(CommandEffect::RateMessage {
                    message_id,
                    message_digest: snapshot.message_digest.clone(),
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

/// Everything rated in this session.
///
/// The reason to read this back is to see what the learning layer will see, so
/// a rating the log has moved out from under is listed and marked rather than
/// quietly omitted.
fn list(snapshot: &FeedbackSnapshot) -> String {
    if snapshot.all.is_empty() {
        return "\nNothing rated in this session yet.\n".to_string();
    }
    let mut out = format!("\nRatings in this session ({}):\n", snapshot.all.len());
    for (message_id, rating, note, current) in &snapshot.all {
        let note = note
            .as_deref()
            .map(|note| format!(" — {note}"))
            .unwrap_or_default();
        let stale = if *current {
            ""
        } else {
            "  (stale: the message at this position has changed)"
        };
        out.push_str(&format!("  [{message_id}] {rating}{note}{stale}\n"));
    }
    out
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
    /// Fingerprint of that message, so a rating survives a compaction without
    /// being reattributed to whatever ends up at the same index.
    pub(crate) message_digest: String,
    /// The rating already on it, if any.
    pub(crate) rating: Option<String>,
    pub(crate) note: Option<String>,
    /// The compare-and-set token to present when changing it.
    pub(crate) version: Option<String>,
    /// Every rating in the session, oldest message id first, as
    /// `(message_id, rating, note, still_describes_that_message)`.
    ///
    /// Read back by `/feedback list`. A rating whose digest no longer matches
    /// the message at its index is listed as stale rather than hidden: it is
    /// still a thing the reader wrote, and silently dropping it from the list
    /// would make a compaction look like it lost work.
    pub(crate) all: Vec<(String, String, Option<String>, bool)>,
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

    #[test]
    fn an_empty_session_lists_nothing_and_says_so() {
        assert!(list(&FeedbackSnapshot::default()).contains("Nothing rated"));
    }

    /// `/feedback list` is how you see what the learning layer will see, so a
    /// rating the log has moved out from under is shown and marked rather than
    /// quietly omitted — otherwise a compaction looks like it lost work.
    #[test]
    fn a_stale_rating_is_listed_and_marked_rather_than_hidden() {
        let snapshot = FeedbackSnapshot {
            all: vec![
                (
                    "3".into(),
                    "positive".into(),
                    Some("nailed it".into()),
                    true,
                ),
                ("7".into(), "negative".into(), None, false),
            ],
            ..FeedbackSnapshot::default()
        };
        let text = list(&snapshot);

        assert!(text.contains("[3] positive — nailed it"), "{text}");
        assert!(text.contains("[7] negative"), "{text}");
        assert!(
            text.contains("stale"),
            "the moved rating is not marked: {text}"
        );
        assert_eq!(
            text.matches("stale").count(),
            1,
            "only the moved rating is stale: {text}"
        );
    }

    /// The note is worth more than the thumb, so it has to be shown back.
    #[test]
    fn an_existing_rating_is_reported_with_its_note() {
        let snapshot = FeedbackSnapshot {
            message_id: Some("7".into()),
            rating: Some("negative".into()),
            note: Some("missed the point".into()),
            version: Some("v1".into()),
            ..FeedbackSnapshot::default()
        };
        let text = describe(&snapshot);
        assert!(text.contains("negative"), "{text}");
        assert!(text.contains("missed the point"), "{text}");
    }
}
