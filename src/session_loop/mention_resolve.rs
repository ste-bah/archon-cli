//! Resolving `@session:` mentions at send time (#200 Phase 4).
//!
//! # Why send time and not type time
//!
//! A session reference is a *snapshot*: `prepare_session_reference` reads the
//! other session's stored log at the instant it is called and freezes an
//! excerpt of it. That makes the moment of the read a real decision, not a
//! plumbing detail.
//!
//! Resolving when the mention is picked would mean the excerpt is as of the
//! keystroke. A user who picks a session, then spends four minutes writing the
//! question — or watches that other session finish the very work they are
//! asking about — would send a snapshot that predates the thing they wanted
//! looked at, with nothing on screen to say so. The stale read is invisible,
//! which is what makes it dangerous.
//!
//! Resolving here, immediately before the turn is composed, makes the excerpt
//! as of send. The consequences are accepted deliberately:
//!
//! - **Failure moves later.** A session deleted between picking and sending
//!   fails at Enter rather than at the pick. That is why every failure here
//!   aborts the whole turn with the reason on screen — a turn that quietly
//!   went out without the context the user attached would be the exact silent
//!   emptiness `/session-ref` was fixed to stop.
//! - **The picker's list may be stale, and it does not matter.** The overlay
//!   only has to produce an id. Nothing it displays is carried into the turn.
//!
//! Only `@session:` tokens are touched. `@`-paths (the `/files` convention)
//! and bare `@`s are ordinary text and are left exactly as typed.

use archon_core::mention::{MentionToken, scan_tokens};
use archon_core::session_reference::{SessionReferenceLimits, prepare_session_reference};

use crate::slash_context::SlashCommandContext;

/// Prepare an untrusted block for every `@session:` token in `input`.
///
/// `Err` carries a sentence for the user and means: do not send this turn.
/// There is no partial success. If one of two references cannot be read, the
/// model would answer a comparison question having seen one side of it, and
/// neither the user nor the model would know which side was missing.
pub(super) async fn resolve_prompt_mentions(
    input: &str,
    cmd_ctx: &SlashCommandContext,
) -> Result<Vec<String>, String> {
    let mut wanted: Vec<String> = Vec::new();
    for token in scan_tokens(input) {
        match token {
            MentionToken::Malformed { span } => {
                return Err(format!(
                    "`{}` names no session. Type `@` and pick one from the list, \
                     or delete the token — nothing was sent.",
                    &input[span]
                ));
            }
            // The same session mentioned twice is one attachment. Injecting
            // the transcript twice would double the cost of the turn and tell
            // the model nothing it did not already have.
            MentionToken::Session { id, .. } if !wanted.contains(&id) => wanted.push(id),
            MentionToken::Session { .. } => {}
        }
    }

    let mut blocks = Vec::with_capacity(wanted.len());
    for id in wanted {
        let snapshot = prepare_session_reference(
            cmd_ctx.session_store.as_ref(),
            &id,
            &cmd_ctx.session_id,
            &cmd_ctx.working_dir,
            SessionReferenceLimits::default(),
        )
        .map_err(|error| {
            tracing::warn!(referenced_session = %id, %error, "@-mention could not be resolved");
            format!("@session:{id} could not be attached: {error}. Nothing was sent.")
        })?;
        blocks.push(snapshot.injectable_text().to_string());
    }
    Ok(blocks)
}

#[cfg(test)]
#[path = "mention_resolve_tests.rs"]
mod tests;
