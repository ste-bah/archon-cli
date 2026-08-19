//! What a recorded exchange looks like on disk (#189 Phase 5).
//!
//! One file per request. JSON rather than a binary format because the point of
//! a cassette is that a human can open it and see what the model was asked and
//! what it said — a recording nobody can read is a recording nobody will trust
//! when a test starts failing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::provider::{LlmError, LlmResponse};
use crate::streaming::StreamEvent;
use crate::types::Usage;

/// A recorded `complete` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedResponse {
    pub content: Vec<serde_json::Value>,
    pub usage: Usage,
    pub stop_reason: String,
}

impl From<&LlmResponse> for RecordedResponse {
    fn from(response: &LlmResponse) -> Self {
        Self {
            content: response.content.clone(),
            usage: response.usage.clone(),
            stop_reason: response.stop_reason.clone(),
        }
    }
}

impl From<RecordedResponse> for LlmResponse {
    fn from(recorded: RecordedResponse) -> Self {
        Self {
            content: recorded.content,
            usage: recorded.usage,
            stop_reason: recorded.stop_reason,
        }
    }
}

/// One request and everything the provider said in reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cassette {
    /// Key this file is stored under. Repeated inside the file so a cassette
    /// that gets renamed can still be checked against its own contents.
    pub digest: String,
    /// Which provider produced it — for the reader, not for matching. Matching
    /// on it would mean a cassette recorded against Anthropic could not be
    /// replayed after a provider rename, for no benefit.
    pub provider: String,
    pub model: String,
    /// The exact bytes that were hashed.
    ///
    /// Kept because the only useful question on a miss is "how does this
    /// request differ from the recorded one", and that cannot be answered from
    /// a hash.
    pub canonical_request: String,
    /// Every event, in order, with the original boundaries.
    ///
    /// Boundaries are part of the recording rather than an accident of it: a
    /// lot of agent-loop behaviour is sensitive to how a response is chunked,
    /// and a replay that delivered one big `TextDelta` would not exercise any
    /// of it.
    #[serde(default)]
    pub events: Vec<StreamEvent>,
    /// Present only when the exchange went through `complete` rather than
    /// `stream`.
    #[serde(default)]
    pub response: Option<RecordedResponse>,
}

impl Cassette {
    pub fn path_in(dir: &Path, digest: &str) -> PathBuf {
        dir.join(format!("{digest}.json"))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read cassette {}: {error}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|error| format!("cassette {} is not readable: {error}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
        let path = Self::path_in(dir, &self.digest);
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("could not serialise a cassette: {error}"))?;
        std::fs::write(&path, text)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok(path)
    }

    /// Rebuild the `complete` response this exchange represents.
    ///
    /// A cassette recorded through `stream` can still answer `complete`, and
    /// the reverse: an agent is free to call either, and refusing to answer
    /// because the recording was made the other way round would make cassettes
    /// depend on a detail of the caller rather than on the request.
    ///
    /// The stream-to-response collapse is the crate's existing
    /// `collect_completion_response` — the same one the Anthropic and Codex
    /// clients use — rather than a second implementation here. A replay whose
    /// idea of "the response these events add up to" differed from the real
    /// providers' would be worse than no replay at all.
    pub async fn as_response(&self) -> Result<LlmResponse, LlmError> {
        if let Some(recorded) = &self.response {
            return Ok(recorded.clone().into());
        }
        if self.events.is_empty() {
            return Err(LlmError::Serialize(format!(
                "cassette {} holds neither a response nor any events",
                self.digest
            )));
        }
        crate::completion_accumulator::collect_completion_response(self.replay_events()).await
    }

    /// The recorded events on a channel, delivered with their original
    /// boundaries.
    ///
    /// Capacity is the whole sequence so the send loop cannot block: a replay
    /// has no producer to apply backpressure to, and a caller that stops
    /// reading should drop the receiver rather than stall a task nobody is
    /// waiting on.
    pub fn replay_events(&self) -> tokio::sync::mpsc::Receiver<StreamEvent> {
        let (tx, rx) = tokio::sync::mpsc::channel(self.events.len().max(1));
        for event in &self.events {
            let _ = tx.try_send(event.clone());
        }
        rx
    }
}

#[cfg(test)]
#[path = "replay_cassette_tests.rs"]
mod tests;
