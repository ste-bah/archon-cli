use serde::Deserialize;

// ── YAML spec types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GameTheorySpec {
    pub version: String,
    pub spec_id: String,
    #[serde(default)]
    pub cost_cap_usd: f64,
    pub tiers: Vec<TierEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TierEntry {
    pub id: u8,
    pub name: String,
    #[serde(default = "default_concurrency")]
    pub concurrency_cap: usize,
    #[serde(default)]
    pub agents: Vec<AgentEntry>,
}

fn default_concurrency() -> usize {
    4
}

#[derive(Debug, Deserialize)]
pub struct AgentEntry {
    pub key: String,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub mandatory: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

// ── RoutingDecision ────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoutingDecision {
    pub run_id: String,
    pub fingerprint_id: String,
    pub enabled_specialists: Vec<String>,
    /// (agent_key, reason)
    pub skipped_specialists: Vec<(String, String)>,
    /// (expression, evaluated_result)
    pub evaluated_conditions: Vec<(String, bool)>,
    pub created_at: String,
}
