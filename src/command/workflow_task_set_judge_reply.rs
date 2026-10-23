//! One whole judge reply (Issue 260): the first answer and, while the output
//! limit cuts it off, its continuations.
//!
//! A truncated reply is never parsed as it stands. It is continued only
//! while its JSON document is open: each continuation must extend the
//! document so that it stays a valid JSON prefix, and a resumed freeze
//! continues the reply saved by an earlier attempt instead of asking again.
//! Progress is the document's parse position moving over at least one token
//! that is not whitespace, never a count of continuations and never two
//! chunks being unequal (a long string repeats text legitimately); a reply
//! past `MAX_PARTIAL_REPLY_BYTES` is no progress. A truncated reply with no
//! document begun (prose only) is not continued: the judge is asked again. A continuation that adds nothing, contradicts the
//! document (restarts it, breaks its syntax) or ends normally with the
//! document still open is no progress: the judge is [`JudgeIncomplete`],
//! resumable. A reply whose document is already complete when it is cut off
//! is not continued (a continuation could restart with another document);
//! it is no usable verdict, and the judge is asked again.

use std::time::Duration;

use anyhow::Result;
use archon_workflow::WorkflowLlmClient;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;

use super::incomplete::JudgeIncompleteKind;
use super::partial::{MAX_PARTIAL_REPLY_BYTES, PartialReply};
use super::{CONTINUE_PROMPT, JUDGE_TIMEOUT_SECS, JudgeIncomplete};

/// How a judge reply ended, read from its finish reason.
pub(super) enum Ending {
    Complete,
    Truncated(String),
    /// The judge declined to judge the content it was shown: its own
    /// outcome about the candidates of this call, never the provider's.
    Refused(String),
    Unsupported(String),
}

pub(super) fn ending(outcome: &WorkflowAgentOutcome) -> Ending {
    match outcome.stop_reason.as_deref() {
        Some("end_turn" | "stop" | "completed") => Ending::Complete,
        Some(reason @ ("max_tokens" | "length")) => Ending::Truncated(reason.to_string()),
        Some(reason @ ("refusal" | "content_filter")) => Ending::Refused(format!(
            "acceptance judge refused to judge these checks (stop reason '{reason}')"
        )),
        Some(reason) => Ending::Unsupported(format!(
            "acceptance judge ended with unsupported stop reason '{reason}'"
        )),
        None => Ending::Unsupported(
            "acceptance judge returned no finish reason; possibly truncated JSON is never parsed"
                .to_string(),
        ),
    }
}

/// How far a reply's JSON document has got, as a parser reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Document {
    /// No document has begun yet (only a preamble).
    NotStarted,
    /// A valid, still open JSON prefix.
    Open,
    /// One whole document.
    Complete,
    /// Not a JSON prefix: the text contradicts itself.
    Invalid,
}

/// The state of the document `reply` holds: it starts at the first `{`.
pub(super) fn document(reply: &str) -> Document {
    let Some(start) = reply.find('{') else {
        return Document::NotStarted;
    };
    let mut stream =
        serde_json::Deserializer::from_str(&reply[start..]).into_iter::<serde::de::IgnoredAny>();
    match stream.next() {
        Some(Ok(_)) => Document::Complete,
        Some(Err(error)) if error.is_eof() => Document::Open,
        None => Document::Open,
        Some(Err(_)) => Document::Invalid,
    }
}

/// A reply that gave no usable verdict and may be asked again: why, and
/// whose it is (the judge's own truncation, or the provider's finish).
pub(super) type Unusable = (String, JudgeIncompleteKind);

/// One whole reply to `task`. `Ok(Ok(reply))` ended normally with a document
/// to judge; `Ok(Err(unusable))` gave no usable verdict and may be asked
/// again; `Err` is [`JudgeIncomplete`] (no progress, a provider error, a
/// timeout).
pub(super) async fn complete_reply(
    client: &dyn WorkflowLlmClient,
    messages: &[serde_json::Value],
    model: &str,
    partial: Option<&PartialReply<'_>>,
) -> Result<std::result::Result<String, Unusable>> {
    let (mut reply, mut chunks) = partial
        .and_then(PartialReply::open_reply)
        .unwrap_or_default();
    let mut continuing = !reply.is_empty();
    loop {
        let mut request = messages.to_vec();
        if continuing {
            request.push(serde_json::json!({ "role": "assistant", "content": reply.clone() }));
            request.push(serde_json::json!({ "role": "user", "content": CONTINUE_PROMPT }));
        }
        let progress = archon_shell::progress::Progress::new(true);
        let outcome = progress
            .bound(
                Duration::from_secs(JUDGE_TIMEOUT_SECS),
                client.send_message_with_progress(
                    request,
                    Vec::new(),
                    Vec::new(),
                    model,
                    0.0,
                    progress.clone(),
                ),
            )
            .await
            .map_err(|_| {
                JudgeIncomplete::provider(format!(
                    "a judge call stalled after {JUDGE_TIMEOUT_SECS}s with no provider progress"
                ))
            })?
            .map_err(|error| {
                JudgeIncomplete::provider(format!("the provider gave no reply: {error}"))
            })?;
        let grown = format!("{reply}{}", outcome.content);
        if grown.len() > MAX_PARTIAL_REPLY_BYTES
            || archon_observability::redaction::redact_text(&grown).len() > MAX_PARTIAL_REPLY_BYTES
        {
            return Err(stalled(
                JudgeIncompleteKind::AuthoredReply,
                &reply,
                &format!(
                    "the reply grew past {MAX_PARTIAL_REPLY_BYTES} bytes, more than any verdict needs"
                ),
            ));
        }
        let state = document(&grown);
        match ending(&outcome) {
            Ending::Unsupported(why) => return Ok(Err((why, JudgeIncompleteKind::Provider))),
            Ending::Refused(why) => {
                if let Some(partial) = partial {
                    partial.spent();
                }
                return Ok(Err((why, JudgeIncompleteKind::Refusal)));
            }
            Ending::Complete if !continuing => return Ok(Ok(grown)),
            Ending::Complete => {
                return match state {
                    Document::Complete => Ok(Ok(grown)),
                    state => Err(stalled(
                        JudgeIncompleteKind::AuthoredReply,
                        &reply,
                        &format!(
                            "the continuation ended normally with the document {}",
                            describe(state)
                        ),
                    )),
                };
            }
            Ending::Truncated(reason) => {
                if state == Document::Complete {
                    // A whole document cut off after it: never continued
                    // (a continuation could restart with another one) and
                    // never accepted as it stands.
                    if let Some(partial) = partial {
                        partial.spent();
                    }
                    return Ok(Err((
                        format!(
                            "the reply was truncated by stop reason '{reason}' after a complete document; a truncated reply is never accepted"
                        ),
                        JudgeIncompleteKind::AuthoredReply,
                    )));
                }
                if !continuing && state != Document::Open {
                    // No document begun (prose only), or not a JSON prefix:
                    // nothing to continue, and no usable verdict.
                    return Ok(Err((
                        format!(
                            "the reply was truncated by stop reason '{reason}' before any JSON document"
                        ),
                        JudgeIncompleteKind::AuthoredReply,
                    )));
                }
                // Progress: the document's parse position moved over at
                // least one token that is not whitespace.
                if outcome.content.trim().is_empty() || state != Document::Open {
                    return Err(stalled(
                        JudgeIncompleteKind::AuthoredReply,
                        &reply,
                        &format!(
                            "the reply was truncated by stop reason '{reason}' and its continuation {}",
                            if outcome.content.trim().is_empty() {
                                "added no token"
                            } else {
                                "contradicts the document it must extend"
                            }
                        ),
                    ));
                }
                reply = grown;
                chunks = chunks.saturating_add(1);
                continuing = true;
                if let Some(partial) = partial
                    && !partial.extended(&reply, chunks, outcome.content.len())
                {
                    // The host could not save a usable reply: its fault.
                    return Err(stalled(
                        JudgeIncompleteKind::Provider,
                        &reply,
                        "the usable redacted continuation could not be persisted",
                    ));
                }
            }
        }
    }
}

fn describe(state: Document) -> &'static str {
    match state {
        Document::NotStarted => "never begun",
        Document::Open => "still open",
        Document::Complete => "complete",
        Document::Invalid => "contradicted",
    }
}

fn stalled(kind: JudgeIncompleteKind, reply: &str, why: &str) -> anyhow::Error {
    JudgeIncomplete::new(
        kind,
        format!(
        "{why} ({} characters held); a truncated verdict is never accepted, and a retry continues the saved reply",
            reply.len()
        ),
    )
    .into()
}
