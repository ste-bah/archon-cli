//! Canonical agent, subagent, tool, and pipeline activity events.
//!
//! The TUI can render these events, logs can persist them, and tests can assert
//! exact lifecycle order without scraping human-facing strings.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::redaction::redact;

/// Lifecycle event categories emitted by Archon runtime surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityKind {
    /// A parent model turn started.
    ParentTurnStarted,
    /// A parent model turn completed.
    ParentTurnCompleted,
    /// A child agent was queued for execution.
    AgentQueued,
    /// A child agent was spawned.
    AgentSpawned,
    /// A child agent is actively running.
    AgentRunning,
    /// A child agent moved into background execution.
    AgentBackgrounded,
    /// A backgrounded child agent resumed.
    AgentResumed,
    /// Execution is waiting on a permission decision.
    AgentWaitingPermission,
    /// Execution is waiting on provider/auth availability.
    AgentWaitingProvider,
    /// A tool call started.
    ToolStarted,
    /// A tool call completed successfully.
    ToolCompleted,
    /// A tool call failed.
    ToolFailed,
    /// A pipeline tier started.
    PipelineTierStarted,
    /// A pipeline specialist started.
    PipelineSpecialistStarted,
    /// A pipeline specialist completed.
    PipelineSpecialistCompleted,
    /// An artifact was created and can be inspected.
    ArtifactCreated,
    /// An agent failed.
    AgentFailed,
    /// An agent completed.
    AgentCompleted,
    /// Execution was cancelled.
    Cancelled,
}

/// Normalized lifecycle state for activity rows and persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityStatus {
    /// Work is queued but not yet running.
    Queued,
    /// Work is actively running.
    Running,
    /// Work is waiting on permission, provider, or another external condition.
    Waiting,
    /// Work has been backgrounded.
    Backgrounded,
    /// Work completed successfully.
    Completed,
    /// Work failed.
    Failed,
    /// Work was cancelled.
    Cancelled,
}

/// Source-of-truth activity event shared across runtime, TUI, logs, and tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentActivityEvent {
    /// Unique event identifier.
    pub event_id: String,
    /// Parent session identifier.
    pub session_id: String,
    /// Optional pipeline/run identifier.
    pub run_id: Option<String>,
    /// Optional parent event or actor identifier.
    pub parent_id: Option<String>,
    /// Optional agent identifier.
    pub agent_id: Option<String>,
    /// Optional subagent identifier.
    pub subagent_id: Option<String>,
    /// Optional agent key.
    pub agent_key: Option<String>,
    /// Optional subagent type.
    pub subagent_type: Option<String>,
    /// Event category.
    pub kind: AgentActivityKind,
    /// Normalized event status.
    pub status: AgentActivityStatus,
    /// Human-facing detail. Must not contain secrets.
    pub message: String,
    /// Optional inspectable artifact identifier.
    pub artifact_id: Option<String>,
    /// Optional provider identifier.
    pub provider: Option<String>,
    /// Optional model identifier.
    pub model: Option<String>,
    /// Optional event-level cost in USD.
    pub cost_usd: Option<f64>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp, when the event represents a mutable activity row.
    pub updated_at: Option<DateTime<Utc>>,
}

impl AgentActivityEvent {
    /// Construct a new event with a generated identifier and current timestamp.
    pub fn new(
        session_id: impl Into<String>,
        kind: AgentActivityKind,
        status: AgentActivityStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.into(),
            run_id: None,
            parent_id: None,
            agent_id: None,
            subagent_id: None,
            agent_key: None,
            subagent_type: None,
            kind,
            status,
            message: message.into(),
            artifact_id: None,
            provider: None,
            model: None,
            cost_usd: None,
            created_at: Utc::now(),
            updated_at: None,
        }
    }

    /// Attach a run identifier.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Attach a parent identifier.
    pub fn with_parent_id(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Attach an agent identifier.
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Attach a subagent identifier.
    pub fn with_subagent_id(mut self, subagent_id: impl Into<String>) -> Self {
        self.subagent_id = Some(subagent_id.into());
        self
    }

    /// Attach an agent key.
    pub fn with_agent_key(mut self, agent_key: impl Into<String>) -> Self {
        self.agent_key = Some(agent_key.into());
        self
    }

    /// Attach a subagent type.
    pub fn with_subagent_type(mut self, subagent_type: impl Into<String>) -> Self {
        self.subagent_type = Some(subagent_type.into());
        self
    }

    /// Attach an artifact identifier.
    pub fn with_artifact_id(mut self, artifact_id: impl Into<String>) -> Self {
        self.artifact_id = Some(artifact_id.into());
        self
    }

    /// Attach provider and model identifiers.
    pub fn with_provider_model(
        mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        self.provider = Some(provider.into());
        self.model = Some(model.into());
        self
    }

    /// Attach event-level cost.
    pub fn with_cost_usd(mut self, cost_usd: f64) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }

    /// Mark the event as updated now.
    pub fn touch(mut self) -> Self {
        self.updated_at = Some(Utc::now());
        self
    }
}

/// Sink interface for activity-event consumers.
pub trait AgentActivitySink: std::fmt::Debug + Send + Sync {
    /// Emit one activity event.
    fn emit(&self, event: AgentActivityEvent);
}

/// Test and local-inspection sink that keeps events in memory.
#[derive(Debug, Clone, Default)]
pub struct InMemoryActivitySink {
    events: Arc<Mutex<Vec<AgentActivityEvent>>>,
}

impl InMemoryActivitySink {
    /// Construct an empty in-memory sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of emitted events in insertion order.
    pub fn events(&self) -> Vec<AgentActivityEvent> {
        self.events
            .lock()
            .expect("activity sink mutex poisoned")
            .clone()
    }

    /// Return the number of emitted events.
    pub fn len(&self) -> usize {
        self.events
            .lock()
            .expect("activity sink mutex poisoned")
            .len()
    }

    /// Return true when no events have been emitted.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AgentActivitySink for InMemoryActivitySink {
    fn emit(&self, event: AgentActivityEvent) {
        self.events
            .lock()
            .expect("activity sink mutex poisoned")
            .push(event);
    }
}

/// JSONL-backed activity sink for durable session timelines.
#[derive(Debug, Clone)]
pub struct JsonlActivitySink {
    path: PathBuf,
    writer_lock: Arc<Mutex<()>>,
}

impl JsonlActivitySink {
    /// Construct a JSONL sink at the given file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            writer_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Return the backing JSONL file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AgentActivitySink for JsonlActivitySink {
    fn emit(&self, event: AgentActivityEvent) {
        if let Err(error) = append_activity_event(&self.path, &self.writer_lock, event) {
            tracing::warn!(
                error = %error,
                path = %self.path.display(),
                "failed to persist activity event"
            );
        }
    }
}

/// Build the canonical session activity JSONL path.
pub fn activity_jsonl_path(base_dir: impl AsRef<Path>, session_id: &str) -> PathBuf {
    base_dir
        .as_ref()
        .join(session_id)
        .join("activity")
        .join("events.jsonl")
}

/// Read all persisted activity events from a JSONL file.
pub fn read_activity_jsonl(path: impl AsRef<Path>) -> anyhow::Result<Vec<AgentActivityEvent>> {
    let file = File::open(path.as_ref())?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        events.push(serde_json::from_str(&line)?);
    }
    Ok(events)
}

fn append_activity_event(
    path: &Path,
    writer_lock: &Mutex<()>,
    event: AgentActivityEvent,
) -> anyhow::Result<()> {
    let _guard = writer_lock
        .lock()
        .expect("activity jsonl writer mutex poisoned");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let redacted = redact_activity_event(event);
    serde_json::to_writer(&mut file, &redacted)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn redact_activity_event(mut event: AgentActivityEvent) -> AgentActivityEvent {
    event.session_id = redact(&event.session_id);
    event.run_id = event.run_id.map(|value| redact(&value));
    event.parent_id = event.parent_id.map(|value| redact(&value));
    event.agent_id = event.agent_id.map(|value| redact(&value));
    event.subagent_id = event.subagent_id.map(|value| redact(&value));
    event.agent_key = event.agent_key.map(|value| redact(&value));
    event.subagent_type = event.subagent_type.map(|value| redact(&value));
    event.message = redact(&event.message);
    event.artifact_id = event.artifact_id.map(|value| redact(&value));
    event.provider = event.provider.map(|value| redact(&value));
    event.model = event.model.map(|value| redact(&value));
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_event_carries_required_context() {
        let event = AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::AgentSpawned,
            AgentActivityStatus::Running,
            "spawned explore",
        )
        .with_run_id("run-1")
        .with_parent_id("parent")
        .with_agent_id("agent-1")
        .with_subagent_id("sub-1")
        .with_agent_key("explore")
        .with_subagent_type("explore")
        .with_artifact_id("artifact-1")
        .with_provider_model("anthropic", "claude")
        .with_cost_usd(0.25)
        .touch();

        assert_eq!(event.session_id, "session-1");
        assert_eq!(event.kind, AgentActivityKind::AgentSpawned);
        assert_eq!(event.status, AgentActivityStatus::Running);
        assert_eq!(event.run_id.as_deref(), Some("run-1"));
        assert_eq!(event.parent_id.as_deref(), Some("parent"));
        assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(event.subagent_id.as_deref(), Some("sub-1"));
        assert_eq!(event.agent_key.as_deref(), Some("explore"));
        assert_eq!(event.subagent_type.as_deref(), Some("explore"));
        assert_eq!(event.artifact_id.as_deref(), Some("artifact-1"));
        assert_eq!(event.provider.as_deref(), Some("anthropic"));
        assert_eq!(event.model.as_deref(), Some("claude"));
        assert_eq!(event.cost_usd, Some(0.25));
        assert!(event.updated_at.is_some());
    }

    #[test]
    fn in_memory_sink_preserves_event_order() {
        let sink = InMemoryActivitySink::new();
        sink.emit(AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::ParentTurnStarted,
            AgentActivityStatus::Running,
            "turn started",
        ));
        sink.emit(AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::ParentTurnCompleted,
            AgentActivityStatus::Completed,
            "turn complete",
        ));

        let events = sink.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, AgentActivityKind::ParentTurnStarted);
        assert_eq!(events[1].kind, AgentActivityKind::ParentTurnCompleted);
    }

    #[test]
    fn sink_trait_object_records_events() {
        let sink = InMemoryActivitySink::new();
        let trait_object: &dyn AgentActivitySink = &sink;

        trait_object.emit(AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::ToolStarted,
            AgentActivityStatus::Running,
            "Read started",
        ));

        assert_eq!(sink.len(), 1);
    }

    #[test]
    fn jsonl_sink_persists_events_for_restart_readback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = activity_jsonl_path(dir.path(), "session-1");
        let sink = JsonlActivitySink::new(path.clone());

        sink.emit(AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::AgentSpawned,
            AgentActivityStatus::Running,
            "spawned researcher",
        ));
        sink.emit(AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::AgentCompleted,
            AgentActivityStatus::Completed,
            "completed researcher",
        ));

        let events = read_activity_jsonl(path).expect("read persisted events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, AgentActivityKind::AgentSpawned);
        assert_eq!(events[1].status, AgentActivityStatus::Completed);
    }

    #[test]
    fn jsonl_sink_redacts_secret_shapes_before_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = activity_jsonl_path(dir.path(), "session-2");
        let sink = JsonlActivitySink::new(path.clone());
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz";

        sink.emit(AgentActivityEvent::new(
            "session-2",
            AgentActivityKind::ToolFailed,
            AgentActivityStatus::Failed,
            format!("provider failed with {secret}"),
        ));

        let raw = std::fs::read_to_string(&path).expect("raw jsonl");
        assert!(!raw.contains(secret));
        assert!(raw.contains("***REDACTED***"));
        let events = read_activity_jsonl(path).expect("read persisted events");
        assert_eq!(events[0].message, "provider failed with ***REDACTED***");
    }

    #[test]
    fn jsonl_reader_ignores_blank_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = activity_jsonl_path(dir.path(), "session-3");
        std::fs::create_dir_all(path.parent().expect("activity dir")).expect("mkdir");
        let event = AgentActivityEvent::new(
            "session-3",
            AgentActivityKind::Cancelled,
            AgentActivityStatus::Cancelled,
            "cancelled",
        );
        std::fs::write(
            &path,
            format!("\n{}\n\n", serde_json::to_string(&event).expect("json")),
        )
        .expect("write jsonl");

        let events = read_activity_jsonl(path).expect("read persisted events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentActivityKind::Cancelled);
    }
}
