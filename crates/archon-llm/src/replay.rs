//! Record and replay LLM exchanges (#189 Phase 5).
//!
//! Testing the agent loop used to require a live provider, so roughly forty
//! tests in this workspace each hand-write their own `LlmProvider` returning a
//! scripted event sequence. Each is a guess about what a provider does; none of
//! them was ever produced by one. A cassette is the alternative — record what a
//! real provider said once, then replay it byte for byte, offline, forever.
//!
//! Two modes, both off unless `ARCHON_LLM_REPLAY` is set:
//!
//! - `record` wraps a real provider, forwards every call, and writes what came
//!   back to a cassette directory.
//! - `replay` serves from that directory and **never** reaches the network. The
//!   real provider is dropped entirely rather than kept as a fallback, so a
//!   miss cannot silently become a live call — a test that quietly stopped
//!   exercising its recorded path would still pass, which is the one failure
//!   this whole mechanism exists to prevent.
//!
//! A miss in replay mode is an error naming the digest, the directory and the
//! command that would record it.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature,
};
use crate::replay_cassette::Cassette;
use crate::replay_digest::{canonical_json, digest};
use crate::streaming::StreamEvent;

/// Selects the mode. Unset or empty means "not in use" and costs nothing.
pub const MODE_ENV: &str = "ARCHON_LLM_REPLAY";
/// Overrides where cassettes live.
pub const DIR_ENV: &str = "ARCHON_LLM_CASSETTES";
/// Default cassette directory, relative to the working directory.
const DEFAULT_DIR: &str = ".archon/cassettes";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayMode {
    /// Forward to the real provider and write down what it said.
    Record,
    /// Serve from disk. Nothing reaches the network.
    Replay,
    /// `ARCHON_LLM_REPLAY` was set to something else.
    ///
    /// Carried as a mode rather than rejected at startup so the complaint
    /// arrives at the first request, attached to the provider that was
    /// misconfigured. Passing the request through instead would answer a
    /// request for replay with a live call, which is the failure mode this
    /// phase exists to remove.
    Invalid(String),
}

impl ReplayMode {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "record" => Self::Record,
            "replay" => Self::Replay,
            other => Self::Invalid(other.to_string()),
        }
    }
}

/// Where cassettes are read from and written to.
pub fn cassette_dir() -> PathBuf {
    std::env::var_os(DIR_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_DIR), PathBuf::from)
}

/// Wrap `provider` if `ARCHON_LLM_REPLAY` asks for it, else hand it back.
///
/// Called from the one place every provider in the binary passes through, so
/// record and replay cover the direct chat path, the session agent and every
/// subagent alike — a switch that only worked on one of those would be a
/// recording of part of a run.
pub fn wrap_if_enabled(provider: Arc<dyn LlmProvider>) -> Arc<dyn LlmProvider> {
    let Some(raw) = std::env::var(MODE_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        return provider;
    };
    let mode = ReplayMode::parse(&raw);
    let dir = cassette_dir();
    tracing::info!(mode = ?mode, dir = %dir.display(), "llm replay is active");
    Arc::new(ReplayProvider::new(mode, dir, Some(provider)))
}

/// An `LlmProvider` backed by a directory of cassettes.
pub struct ReplayProvider {
    mode: ReplayMode,
    dir: PathBuf,
    /// The real provider. `None` in replay mode — dropped rather than held, so
    /// there is no path from a miss to the network even by mistake.
    inner: Option<Arc<dyn LlmProvider>>,
}

impl ReplayProvider {
    pub fn new(mode: ReplayMode, dir: PathBuf, inner: Option<Arc<dyn LlmProvider>>) -> Self {
        let inner = if mode == ReplayMode::Record {
            inner
        } else {
            None
        };
        Self { mode, dir, inner }
    }

    /// A provider that only ever reads from `dir`.
    pub fn replaying(dir: PathBuf) -> Self {
        Self::new(ReplayMode::Replay, dir, None)
    }

    /// A provider that forwards to `inner` and writes cassettes into `dir`.
    pub fn recording(inner: Arc<dyn LlmProvider>, dir: PathBuf) -> Self {
        Self::new(ReplayMode::Record, dir, Some(inner))
    }

    fn real(&self) -> Result<&Arc<dyn LlmProvider>, LlmError> {
        match &self.mode {
            ReplayMode::Invalid(value) => Err(LlmError::Unsupported(format!(
                "{MODE_ENV} is set to {value:?}; expected \"record\" or \"replay\". \
                 Refusing to call a provider rather than guess which was meant."
            ))),
            _ => self.inner.as_ref().ok_or_else(|| {
                LlmError::Unsupported(format!(
                    "{MODE_ENV}=record needs a real provider to record from, and none was given"
                ))
            }),
        }
    }

    fn load(&self, request: &LlmRequest) -> Result<Cassette, LlmError> {
        if let ReplayMode::Invalid(value) = &self.mode {
            return Err(LlmError::Unsupported(format!(
                "{MODE_ENV} is set to {value:?}; expected \"record\" or \"replay\""
            )));
        }
        let key = digest(request);
        let path = Cassette::path_in(&self.dir, &key);
        if !path.exists() {
            return Err(LlmError::Unsupported(miss_message(
                &key, &self.dir, request,
            )));
        }
        Cassette::load(&path).map_err(LlmError::Serialize)
    }

    fn write(&self, request: &LlmRequest, cassette: Cassette) {
        match cassette.save(&self.dir) {
            Ok(path) => tracing::info!(
                cassette = %path.display(),
                model = %request.model,
                "recorded an llm exchange"
            ),
            // Warn rather than fail the call. Recording runs against a live
            // provider that has already answered and already been paid for;
            // turning a disk problem into a failed turn would lose the answer
            // as well as the recording.
            Err(error) => tracing::warn!(%error, "could not write a cassette"),
        }
    }

    fn cassette_for(&self, request: &LlmRequest, events: Vec<StreamEvent>) -> Cassette {
        Cassette {
            digest: digest(request),
            provider: self
                .inner
                .as_ref()
                .map_or("unknown", |p| p.name())
                .to_string(),
            model: request.model.clone(),
            canonical_request: canonical_json(request),
            events,
            response: None,
        }
    }
}

/// What a miss says.
///
/// Names the digest so the file can be found, the directory so a wrong one is
/// obvious, the model so a mismatched alias stands out, and how to record it —
/// because "no cassette" with no next step is where this gets abandoned.
fn miss_message(key: &str, dir: &std::path::Path, request: &LlmRequest) -> String {
    format!(
        "no cassette {key} in {} (model {}, {} messages). \
         Nothing was sent: {MODE_ENV}=replay never reaches the network. \
         Record it with {MODE_ENV}=record {DIR_ENV}={} against a live provider.",
        dir.display(),
        request.model,
        request.messages.len(),
        dir.display()
    )
}

#[async_trait]
impl LlmProvider for ReplayProvider {
    fn name(&self) -> &str {
        "replay"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.inner
            .as_ref()
            .map(|inner| inner.models())
            .unwrap_or_default()
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        if self.mode != ReplayMode::Record {
            return Ok(self.load(&request)?.replay_events());
        }

        let mut live = self.real()?.stream(request.clone()).await?;
        // Drained here rather than forwarded chunk by chunk: the recording has
        // to be the whole sequence, and a caller that drops the receiver
        // half-way would otherwise leave a truncated cassette on disk that
        // replays as a complete one.
        let mut events = Vec::new();
        while let Some(event) = live.recv().await {
            events.push(event);
        }
        let cassette = self.cassette_for(&request, events);
        self.write(&request, cassette.clone());
        Ok(cassette.replay_events())
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        if self.mode != ReplayMode::Record {
            return self.load(&request)?.as_response().await;
        }

        let response = self.real()?.complete(request.clone()).await?;
        let mut cassette = self.cassette_for(&request, Vec::new());
        cassette.response = Some((&response).into());
        self.write(&request, cassette);
        Ok(response)
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        // In replay the answer has to come from the recording's own provider,
        // and there is none to ask — but reporting "unsupported" for everything
        // would change what the agent loop does compared with the run that was
        // recorded. Recording forwards; replay claims support, because the
        // recorded events already prove whatever was used.
        self.inner
            .as_ref()
            .is_none_or(|inner| inner.supports_feature(feature))
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        // Recording reaches whatever the real provider reaches, and must not
        // claim otherwise: a policy gate that let a recording run past a cloud
        // restriction would be a hole. Replay reads a local directory.
        self.inner
            .as_ref()
            .map_or(DataFlowClassification::Local, |inner| {
                inner.data_flow_classification()
            })
    }

    fn resolve_alias(&self, alias: &str) -> Option<String> {
        // Resolution has to match the recorded run, or the model name feeding
        // the digest differs and nothing hits.
        self.inner
            .as_ref()
            .and_then(|inner| inner.resolve_alias(alias))
    }
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
