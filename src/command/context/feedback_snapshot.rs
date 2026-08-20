//! The `/feedback` dispatch-site snapshot (#193 Phase C).
//!
//! Split out of `builder.rs` for the 500-line ceiling, the same reason
//! `permissions.rs` holds its own snapshot builder. `build_command_context`
//! calls this only when the primary resolves to `/feedback` or `/rate`: it
//! reads the message log and the sidecar relation, and no other command wants
//! either.

use crate::command::feedback::FeedbackSnapshot;
use crate::slash_context::SlashCommandContext;

/// The last assistant message and any rating on it.
///
/// The message id is its index in the log, which is already the storage key —
/// inventing a second identifier would mean two ways to name one message and a
/// mapping to keep honest between them.
///
/// The scan runs backwards because the message being rated is nearly always the
/// last one, and a session log can be long. A message whose content is not JSON,
/// or carries no role, is skipped rather than guessed at.
pub(super) fn build_feedback_snapshot(slash_ctx: &SlashCommandContext) -> FeedbackSnapshot {
    let Ok(messages) = slash_ctx.session_store.load_messages(&slash_ctx.session_id) else {
        return FeedbackSnapshot::default();
    };
    let Some(index) = messages
        .iter()
        .rposition(|content| is_assistant_message(content))
    else {
        // Nothing to rate, but ratings from before a `/clear` or a rewind may
        // still be there, and `/feedback list` is how you find that out.
        return FeedbackSnapshot {
            all: all_ratings(slash_ctx, &messages),
            ..FeedbackSnapshot::default()
        };
    };

    let message_id = index.to_string();
    let digest = archon_session::feedback::message_digest(&messages[index]);
    let existing = slash_ctx
        .session_store
        .feedback(&slash_ctx.session_id, &message_id)
        .ok()
        .flatten();

    // A rating is keyed by position, and positions move: compaction replaces
    // the whole message list with a shorter one, so a rating left keyed to
    // index 7 would be reported as describing whatever is at 7 afterwards. If
    // the digest disagrees, the rating is not about this message and is
    // reported as absent — but its version is kept, so re-rating overwrites the
    // stale row rather than colliding with it.
    let describes_this_message = existing
        .as_ref()
        .is_some_and(|found| found.message_digest == digest);

    FeedbackSnapshot {
        message_id: Some(message_id),
        message_digest: digest,
        rating: existing
            .as_ref()
            .filter(|_| describes_this_message)
            .map(|found| found.rating.as_str().to_string()),
        note: existing
            .as_ref()
            .filter(|_| describes_this_message)
            .and_then(|found| found.note.clone()),
        version: existing.map(|found| found.version),
        all: all_ratings(slash_ctx, &messages),
    }
}

/// Whether one stored message is an assistant turn.
fn is_assistant_message(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get("role")
                .and_then(serde_json::Value::as_str)
                .map(|role| role == "assistant")
        })
        .unwrap_or(false)
}

/// Every rating in this session, each marked with whether it still describes
/// the message at its index.
///
/// Read back by `/feedback list`. A rating the log has moved out from under is
/// listed and marked rather than omitted: it is still something the reader
/// wrote, and dropping it silently would make a compaction look like it lost
/// work.
fn all_ratings(
    slash_ctx: &SlashCommandContext,
    messages: &[String],
) -> Vec<(String, String, Option<String>, bool)> {
    let Ok(all) = slash_ctx.session_store.all_feedback(&slash_ctx.session_id) else {
        return Vec::new();
    };
    all.into_iter()
        .map(|entry| {
            let current = entry
                .message_id
                .parse::<usize>()
                .ok()
                .and_then(|index| messages.get(index))
                .is_some_and(|content| {
                    archon_session::feedback::message_digest(content) == entry.message_digest
                });
            (
                entry.message_id,
                entry.rating.as_str().to_string(),
                entry.note,
                current,
            )
        })
        .collect()
}
