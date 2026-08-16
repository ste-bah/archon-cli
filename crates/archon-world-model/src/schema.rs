use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identifier for the archon build currently running, as `<version>+<commit>`.
///
/// The commit is what actually identifies behaviour: `CARGO_PKG_VERSION` moves
/// only on release, so a whole development period would otherwise carry one
/// label and a corpus spanning it could not be segmented. Falls back to
/// `<version>+unknown` when built outside a git checkout.
pub fn build_stamp() -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("ARCHON_BUILD_SHA"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl EvidenceRef {
    pub fn new(source: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            id: id.into(),
            path: None,
            hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingRef {
    pub embedding_id: String,
    pub provider: String,
    pub model: String,
    pub source_dimensions: usize,
    pub projection_dimensions: usize,
    pub source_hash: String,
    pub projection_version: String,
    pub redaction_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum WorldTraceSource {
    #[default]
    ActivityEvent,
    PipelineBundle,
    ProviderRuntime,
    Plan,
    Conversation,
    AgentTranscript,
    AgentOutput,
    Workflow,
    Retrospective,
    Memory,
    AgentEvolution,
    ReasoningQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum WorldActionKind {
    AgentAttempt,
    ProviderCall,
    ToolCall,
    PlanUpdate,
    MemorySurface,
    Verification,
    Retry,
    Resume,
    // Coordination verbs (#184 M9). Subagent activity already reached learning
    // through transcripts and activity events, but the verbs multi-agent
    // coordination introduced had no representation — so the learning systems
    // were blind to exactly the behaviour that issue created.
    /// One agent sent another a message.
    MessageSend,
    /// An agent took a task, or declared what it would write.
    TaskClaim,
    /// Work passed from one agent to another.
    Handoff,
    /// An isolated agent's branch was merged back, or discarded.
    ///
    /// The highest-value one: git merge results are ground truth, so these rows
    /// are labelled deterministically with no heuristic labeler involved.
    WorktreeMerge,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WorldLabelSet {
    pub success: Option<bool>,
    pub failure: bool,
    pub retry: bool,
    pub provider_incident: bool,
    pub verification_needed: bool,
    pub user_correction: bool,
    pub plan_drift: bool,
    pub high_cost: bool,
    pub slow_run: bool,
    /// The merge back into the base branch conflicted (#184 M9).
    ///
    /// Ground truth from git, not a judgement: it is the outcome of an actual
    /// merge. That makes it the one label here a model can be trained against
    /// without trusting a labeler.
    pub merge_conflict: bool,
    /// This agent's declared writes overlapped a running agent's at spawn time.
    pub claim_overlap: bool,
    /// The agent ran in its own worktree rather than the shared tree.
    pub isolated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ScalarFeatures {
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub attempt_index: Option<u32>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub quality_overall: Option<f64>,
    pub provider_cooldown_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldTraceRow {
    pub row_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_attempt_id: Option<String>,
    pub session_id: String,
    pub run_id: Option<String>,
    /// Groups the rows produced by agents coordinating with each other (#184 M9).
    ///
    /// Separate from `run_id`, which is already three things depending on who
    /// wrote the row — a real run id, a plan id, a synthesised phase-ordinal —
    /// and overloading it a fourth time would make every existing consumer
    /// wrong. This one has a single meaning: the team a set of agents belongs
    /// to, or the lead that spawned them when there is no team.
    ///
    /// `None` on every row written outside a coordinated run, which is most of
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination_run_id: Option<String>,
    pub source: WorldTraceSource,
    pub action_kind: WorldActionKind,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub state_embedding: Option<EmbeddingRef>,
    pub action_embedding: Option<EmbeddingRef>,
    pub next_state_embedding: Option<EmbeddingRef>,
    pub scalar_features: ScalarFeatures,
    pub labels: WorldLabelSet,
    pub evidence_refs: Vec<EvidenceRef>,
    pub redacted_excerpt: Option<String>,
    /// Archon build that produced this row.
    ///
    /// A corpus collected while archon itself is changing is non-stationary —
    /// the system generating the traces mutates underneath them — and a model
    /// trained across that drift learns the churn rather than the task. Tagging
    /// at write time lets a mixed corpus be filtered afterwards instead of
    /// being discovered unusable later.
    ///
    /// `Option` with a serde default so rows written before this field existed
    /// still deserialize; they read as `None`, which is itself the useful
    /// signal that their provenance is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archon_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Default for WorldTraceRow {
    fn default() -> Self {
        Self {
            row_id: String::new(),
            action_attempt_id: None,
            session_id: String::new(),
            run_id: None,
            coordination_run_id: None,
            source: WorldTraceSource::default(),
            action_kind: WorldActionKind::default(),
            provider: None,
            model: None,
            agent: None,
            state_embedding: None,
            action_embedding: None,
            next_state_embedding: None,
            scalar_features: ScalarFeatures::default(),
            labels: WorldLabelSet::default(),
            evidence_refs: Vec::new(),
            redacted_excerpt: None,
            // Left unset in Default so a row built without `new()` is honestly
            // marked as unknown-provenance rather than silently claiming the
            // running build.
            archon_version: None,
            created_at: Utc::now(),
        }
    }
}

impl WorldTraceRow {
    pub fn new(session_id: impl Into<String>, action_kind: WorldActionKind) -> Self {
        Self {
            row_id: format!("world-row-{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            action_kind,
            // Stamped here rather than at the call sites so every construction
            // path is tagged and none can be forgotten.
            archon_version: Some(build_stamp()),
            created_at: Utc::now(),
            ..Self::default()
        }
    }

    pub fn with_action_attempt_id(mut self, action_attempt_id: impl Into<String>) -> Self {
        self.action_attempt_id = Some(action_attempt_id.into());
        self
    }

    pub fn with_evidence(mut self, evidence: EvidenceRef) -> Self {
        self.evidence_refs.push(evidence);
        self
    }

    pub fn with_row_id(mut self, row_id: impl Into<String>) -> Self {
        self.row_id = row_id.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_trace_row_sets_stable_prefix() {
        let row = WorldTraceRow::new("session-1", WorldActionKind::AgentAttempt);
        assert!(row.row_id.starts_with("world-row-"));
        assert_eq!(row.session_id, "session-1");
        assert_eq!(row.action_kind, WorldActionKind::AgentAttempt);
    }
}

#[cfg(test)]
mod archon_version_tests {
    use super::*;

    /// Every row built through the constructor carries the build that made it,
    /// so a mixed corpus can be filtered rather than discarded.
    #[test]
    fn new_rows_are_stamped_with_the_running_build() {
        let row = WorldTraceRow::new("session-1", WorldActionKind::Unknown);
        assert_eq!(row.archon_version.as_deref(), Some(build_stamp().as_str()));
        // The commit is the part that makes a corpus segmentable; the release
        // version alone would collapse a whole development period to one label.
        assert!(row.archon_version.unwrap().contains('+'));
    }

    /// Rows written before the field existed must still load, and must read as
    /// unknown provenance rather than being attributed to the running build.
    #[test]
    fn rows_without_the_field_deserialize_as_unknown_provenance() {
        let mut value =
            serde_json::to_value(WorldTraceRow::new("session-1", WorldActionKind::Unknown))
                .expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .remove("archon_version");
        let row: WorldTraceRow = serde_json::from_value(value).expect("deserialize legacy row");
        assert_eq!(row.archon_version, None);
    }
}
