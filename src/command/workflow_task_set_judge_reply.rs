//! One whole judge reply (Issue 260): the first answer and, while the output
//! limit cuts it off, its continuations.
//!
//! A truncated reply is never parsed as it stands. It is continued only
//! while its JSON document is open, or when a length stop comes before its
//! first token: each continuation must extend the
//! document so that it stays a valid JSON prefix, and a resumed freeze
//! continues the reply saved by an earlier attempt instead of asking again.
//! Progress is the document's parse position moving over at least one token
//! that is not whitespace, never a count of continuations and never two
//! chunks being unequal (a long string repeats text legitimately); a reply
//! past `MAX_PARTIAL_REPLY_BYTES` is no progress. A continuation that adds
//! nothing, contradicts the
//! document (restarts it, breaks its syntax) or ends normally with the
//! document still open is no progress: the judge is [`JudgeIncomplete`],
//! resumable. A reply whose document is already complete when it is cut off
//! is not continued (a continuation could restart with another document);
//! it is no usable verdict, and the judge is asked again. So is a first
//! reply cut off whose text already contradicts a JSON document.

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

/// What one more chunk makes of a reply (shared with the fidelity critic,
/// `topology_lint/fidelity_critic.rs`): `reply` is the text so far,
/// `continuing` whether the chunk in `outcome` continues it.
pub(crate) enum Step {
    /// One whole document, ended normally.
    Done(String),
    /// Cut off with its document still open: continue with this reply.
    Continue(String),
    /// No usable verdict; it may be asked again. `spent` when a saved
    /// partial reply must not be continued again.
    Unusable {
        why: String,
        kind: JudgeIncompleteKind,
        spent: bool,
    },
    /// A continuation that made no progress: why.
    Stalled(JudgeIncompleteKind, String),
}

/// The next [`Step`] of `reply` given `outcome` (see the module docs).
pub(crate) fn step(reply: &str, continuing: bool, outcome: &WorkflowAgentOutcome) -> Step {
    let grown = format!("{reply}{}", outcome.content);
    if grown.len() > MAX_PARTIAL_REPLY_BYTES
        || archon_observability::redaction::redact_text(&grown).len() > MAX_PARTIAL_REPLY_BYTES
    {
        return Step::Stalled(
            JudgeIncompleteKind::AuthoredReply,
            format!(
                "the reply grew past {MAX_PARTIAL_REPLY_BYTES} bytes, more than any verdict needs"
            ),
        );
    }
    let state = document(&grown);
    let unusable = |why, kind, spent| Step::Unusable { why, kind, spent };
    match ending(outcome) {
        Ending::Unsupported(why) => unusable(why, JudgeIncompleteKind::Provider, false),
        Ending::Refused(why) => unusable(why, JudgeIncompleteKind::Refusal, true),
        Ending::Complete if !continuing => Step::Done(grown),
        Ending::Complete => match state {
            Document::Complete => Step::Done(grown),
            state => Step::Stalled(
                JudgeIncompleteKind::AuthoredReply,
                format!(
                    "the continuation ended normally with the document {}",
                    describe(state)
                ),
            ),
        },
        // A whole document cut off after it: never continued (a
        // continuation could restart with another one) and never accepted
        // as it stands.
        Ending::Truncated(reason) if state == Document::Complete => unusable(
            format!(
                "the reply was truncated by stop reason '{reason}' after a complete document; a truncated reply is never accepted"
            ),
            JudgeIncompleteKind::AuthoredReply,
            true,
        ),
        // The provider's stop reason takes precedence over the parser: a
        // length stop may happen before it has emitted the opening brace.
        Ending::Truncated(_) if !continuing && state == Document::NotStarted => {
            Step::Continue(grown)
        }
        // A first reply that already contradicts a JSON document cannot be
        // extended into one: no usable verdict, asked afresh.
        Ending::Truncated(reason) if !continuing && state == Document::Invalid => unusable(
            format!(
                "the reply was truncated by stop reason '{reason}' with text that is not a JSON document"
            ),
            JudgeIncompleteKind::AuthoredReply,
            false,
        ),
        // Progress: the document's parse position moved over at least one
        // token that is not whitespace.
        Ending::Truncated(reason)
            if outcome.content.trim().is_empty() || state != Document::Open =>
        {
            Step::Stalled(
                JudgeIncompleteKind::AuthoredReply,
                format!(
                    "the reply was truncated by stop reason '{reason}' and its continuation {}",
                    if outcome.content.trim().is_empty() {
                        "added no token"
                    } else {
                        "contradicts the document it must extend"
                    }
                ),
            )
        }
        Ending::Truncated(_) => Step::Continue(grown),
    }
}

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
                JudgeIncomplete::host_error(crate::command::workflow_host_fault::transient(
                    format!(
                        "a judge call stalled after {JUDGE_TIMEOUT_SECS}s with no provider progress"
                    ),
                ))
            })?
            .map_err(JudgeIncomplete::provider_error)?;
        match step(&reply, continuing, &outcome) {
            Step::Done(whole) => return Ok(Ok(whole)),
            Step::Unusable { why, kind, spent } => {
                if spent && let Some(partial) = partial {
                    partial.spent();
                }
                return Ok(Err((why, kind)));
            }
            Step::Stalled(kind, why) => return Err(stalled(kind, &reply, &why)),
            Step::Continue(grown) => {
                reply = grown;
                chunks = chunks.saturating_add(1);
                continuing = true;
                if let Some(partial) = partial
                    && !partial
                        .try_extended(&reply, chunks, outcome.content.len())
                        .map_err(JudgeIncomplete::host_error)?
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
