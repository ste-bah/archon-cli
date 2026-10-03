//! One whole judge reply (Issue 260): the first answer and, while the output
//! limit cuts it off, its continuations.
//!
//! A truncated reply is never parsed as it stands. It is continued only
//! while its JSON document is still open: each continuation must extend the
//! document so that it stays a valid JSON prefix, and a resumed freeze
//! continues the reply saved by an earlier attempt instead of asking again.
//! Progress is the document growing as a parser reads it, never a count of
//! continuations and never two chunks being unequal (a long string repeats
//! text legitimately). A continuation that adds nothing, contradicts the
//! document (restarts it, breaks its syntax) or ends normally with the
//! document still open is no progress: the judge is [`JudgeIncomplete`],
//! resumable. A reply whose document is already complete when it is cut off
//! is not continued (a continuation could restart with another document);
//! it is no usable verdict, and the judge is asked again.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use archon_workflow::WorkflowLlmClient;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;

use super::{CONTINUE_PROMPT, JUDGE_TIMEOUT_SECS, JudgeIncomplete};
use crate::command::workflow_freeze_budget::FreezeProgress;

/// How a judge reply ended, read from its finish reason.
pub(super) enum Ending {
    Complete,
    Truncated(String),
    Unsupported(String),
}

pub(super) fn ending(outcome: &WorkflowAgentOutcome) -> Ending {
    match outcome.stop_reason.as_deref() {
        Some("end_turn" | "stop" | "completed") => Ending::Complete,
        Some(reason @ ("max_tokens" | "length")) => Ending::Truncated(reason.to_string()),
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

/// A batch's partial reply, saved under the freeze cache for a retry, with
/// the count of continuation chunks every attempt saved (the progress the
/// host executor reads; it never goes back).
pub(crate) struct PartialReply<'a> {
    path: PathBuf,
    key: String,
    progress: &'a FreezeProgress,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Saved {
    schema: u32,
    key: String,
    reply: String,
    chunks: u64,
}

const SCHEMA: u32 = 1;

impl<'a> PartialReply<'a> {
    pub(crate) fn new(dir: &Path, key: &str, progress: &'a FreezeProgress) -> Self {
        Self {
            path: dir.join(format!("partial-{key}.json")),
            key: key.to_string(),
            progress,
        }
    }

    fn load(&self) -> Saved {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .filter(|saved| saved.schema == SCHEMA && saved.key == self.key)
            .unwrap_or_default()
    }

    fn store(&self, saved: &Saved) {
        let staging = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let written = self
            .path
            .parent()
            .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
            && serde_json::to_vec(saved).is_ok_and(|bytes| std::fs::write(&staging, bytes).is_ok())
            && std::fs::rename(&staging, &self.path).is_ok();
        if !written {
            let _ = std::fs::remove_file(&staging);
            eprintln!(
                "the judge's partial reply could not be saved at {}; a retry continues from less",
                self.path.display()
            );
        }
    }

    /// Counts the chunks earlier attempts saved, once per freeze attempt.
    pub(crate) fn count_saved(&self) {
        self.progress.reused_judged(self.load().chunks);
    }

    /// The reply to continue, if an earlier attempt left one open.
    fn open_reply(&self) -> Option<String> {
        let saved = self.load();
        (!saved.reply.is_empty()
            && matches!(
                document(&saved.reply),
                Document::Open | Document::NotStarted
            ))
        .then_some(saved.reply)
    }

    /// Saves `reply`, extended by one more chunk, and reports the progress.
    fn extended(&self, reply: &str) {
        let mut saved = self.load();
        saved.schema = SCHEMA;
        saved.key = self.key.clone();
        saved.reply = reply.to_string();
        saved.chunks += 1;
        self.store(&saved);
        self.progress.saved(true);
    }

    /// The saved reply is spent (complete, or no usable verdict): the next
    /// ask starts afresh. The chunk count stays, so progress never goes back.
    pub(crate) fn spent(&self) {
        let mut saved = self.load();
        if !saved.reply.is_empty() {
            saved.reply.clear();
            self.store(&saved);
        }
    }
}

/// One whole reply to `task`. `Ok(Ok(reply))` ended normally with a document
/// to judge; `Ok(Err(why))` gave no usable verdict and may be asked again;
/// `Err` is [`JudgeIncomplete`] (no progress, a provider error, a timeout).
pub(super) async fn complete_reply(
    client: &dyn WorkflowLlmClient,
    task: &str,
    model: &str,
    partial: Option<&PartialReply<'_>>,
) -> Result<std::result::Result<String, String>> {
    let mut reply = partial
        .and_then(PartialReply::open_reply)
        .unwrap_or_default();
    let mut continuing = !reply.is_empty();
    loop {
        let mut messages = vec![serde_json::json!({ "role": "user", "content": task })];
        if continuing {
            messages.push(serde_json::json!({ "role": "assistant", "content": reply.clone() }));
            messages.push(serde_json::json!({ "role": "user", "content": CONTINUE_PROMPT }));
        }
        let outcome = tokio::time::timeout(
            Duration::from_secs(JUDGE_TIMEOUT_SECS),
            client.send_message_with_temperature(messages, Vec::new(), Vec::new(), model, 0.0),
        )
        .await
        .map_err(|_| {
            JudgeIncomplete(format!(
                "a judge call timed out after {JUDGE_TIMEOUT_SECS}s with no reply"
            ))
        })?
        .map_err(|error| JudgeIncomplete(format!("the provider gave no reply: {error}")))?;
        let grown = format!("{reply}{}", outcome.content);
        match ending(&outcome) {
            Ending::Unsupported(why) => return Ok(Err(why)),
            Ending::Complete if !continuing => return Ok(Ok(grown)),
            Ending::Complete => {
                return match document(&grown) {
                    Document::Complete => Ok(Ok(grown)),
                    state => Err(stalled(
                        &reply,
                        &format!(
                            "the continuation ended normally with the document {}",
                            describe(state)
                        ),
                    )),
                };
            }
            Ending::Truncated(reason) => {
                let state = document(&grown);
                if state == Document::Complete {
                    // A whole document cut off after it: never continued
                    // (a continuation could restart with another one) and
                    // never accepted as it stands.
                    if let Some(partial) = partial {
                        partial.spent();
                    }
                    return Ok(Err(format!(
                        "the reply was truncated by stop reason '{reason}' after a complete document; a truncated reply is never accepted"
                    )));
                }
                if !continuing && state == Document::Invalid {
                    // Not even the start of a JSON document: nothing to
                    // continue, and no usable verdict; asked again.
                    return Ok(Err(format!(
                        "the reply was truncated by stop reason '{reason}' and is not a JSON document"
                    )));
                }
                if outcome.content.is_empty() || state == Document::Invalid {
                    return Err(stalled(
                        &reply,
                        &format!(
                            "the reply was truncated by stop reason '{reason}' and its continuation {}",
                            if outcome.content.is_empty() {
                                "added nothing"
                            } else {
                                "contradicts the document it must extend"
                            }
                        ),
                    ));
                }
                reply = grown;
                continuing = true;
                if let Some(partial) = partial {
                    partial.extended(&reply);
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

fn stalled(reply: &str, why: &str) -> anyhow::Error {
    JudgeIncomplete(format!(
        "{why} ({} characters held); a truncated verdict is never accepted, and a retry continues the saved reply",
        reply.len()
    ))
    .into()
}
