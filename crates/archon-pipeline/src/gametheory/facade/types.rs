use std::collections::{BTreeMap, HashMap};

use super::super::fingerprint::GameTheoryFingerprint;
use super::super::routing::RoutingDecision;
use super::memory_context::MemoryRecallAudit;

/// Runtime controls for a full game-theory run.
#[derive(Debug, Clone)]
pub struct GameTheoryRunOptions {
    pub budget_usd: f64,
    pub max_concurrent: usize,
    pub style_profile_id: Option<String>,
    pub enable_tier11: bool,
    pub kb_pack_id: Option<String>,
}

impl Default for GameTheoryRunOptions {
    fn default() -> Self {
        Self {
            budget_usd: 20.0,
            max_concurrent: 4,
            style_profile_id: None,
            enable_tier11: false,
            kb_pack_id: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct SpecialistExecutionOutcome {
    pub(super) outputs: HashMap<String, String>,
    pub(super) failed: Vec<(String, String)>,
    pub(super) memory_audits: Vec<MemoryRecallAudit>,
    pub(super) costs_usd: HashMap<String, f64>,
    pub(super) total_cost_usd: f64,
    pub(super) tier_costs_usd: BTreeMap<u8, f64>,
    pub(super) budget_exceeded: bool,
    pub(super) max_observed_concurrent: usize,
}

#[derive(Debug, Clone)]
pub(super) struct Tier1AgentOutput {
    pub(super) agent_key: String,
    pub(super) content: String,
}

#[derive(Debug, Clone)]
pub(super) struct SpecialistCallOutput {
    pub(super) agent_key: String,
    pub(super) output: Option<String>,
    pub(super) error: Option<String>,
    pub(super) audit: MemoryRecallAudit,
    pub(super) cost_usd: f64,
}

/// Result of a full pipeline run.
#[derive(Debug, Clone)]
pub struct FullPipelineResult {
    pub run_id: String,
    pub fingerprint: GameTheoryFingerprint,
    pub routing_decision: RoutingDecision,
    pub report: String,
    pub specialist_count: usize,
    /// Specialists that failed during execution (agent_key, error_message).
    pub failed_specialists: Vec<(String, String)>,
    /// Per-agent memory recall evidence collected during real LLM execution.
    pub memory_recall: Vec<MemoryRecallAudit>,
    /// Total estimated model cost for successful specialist calls.
    pub total_cost_usd: f64,
    /// Per-specialist estimated model cost.
    pub specialist_costs_usd: HashMap<String, f64>,
    /// Per-tier estimated model cost, keyed by game-theory tier.
    pub tier_costs_usd: BTreeMap<u8, f64>,
    /// Maximum observed specialist concurrency for this run.
    pub max_observed_concurrent: usize,
    /// Overall pipeline status: "completed" (all specialists succeeded) or "partial" (some failed).
    pub status: String,
}

/// Result of replaying one specialist against a stored Tier 1 fingerprint.
#[derive(Debug, Clone)]
pub struct ReplaySpecialistResult {
    pub run_id: String,
    pub agent_key: String,
    pub status: String,
    pub output_summary: String,
    pub cost_usd: f64,
    pub memory_recall: Vec<MemoryRecallAudit>,
}

#[derive(Debug, Clone)]
pub struct InProgressRun {
    pub run_id: String,
    pub situation: String,
    pub started_at: String,
}

#[derive(Debug, Clone)]
pub struct ResumeRunResult {
    pub run_id: String,
    pub resumed_specialists: usize,
    pub skipped_completed_specialists: usize,
    pub failed_specialists: usize,
    pub status: String,
    pub total_cost_usd: f64,
    pub report_words: usize,
}

#[derive(Debug, Clone)]
pub(super) struct StoredRunState {
    pub(super) situation: String,
    pub(super) started_at: String,
    pub(super) status: String,
}
