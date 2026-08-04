//! `[topology]` — milestone 3 guardrail admission.
//!
//! Every invariant is individually disableable and every one defaults to on.
//! Nothing here is read on the hot path: the config is resolved once at session
//! start and admission works from the resolved values.

use serde::{Deserialize, Serialize};

/// How far the ungated-irreversible invariant reaches.
///
/// Serialised as `off` / `where_declared` / `always`. See
/// `archon_topology::live::GateEnforcement` for why `where_declared` is the
/// default rather than the literal reading the design asked for — briefly, the
/// literal reading blocks every irreversible action in every session that never
/// declared a graph, which is every ordinary coding turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GateEnforcementConfig {
    /// Never block on this invariant.
    Off,
    /// Block only where the session's structure declares at least one gate.
    #[default]
    WhereDeclared,
    /// Block whenever no passed gate dominates the action, gates declared or
    /// not.
    Always,
}

/// Guardrail admission configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopologyConfig {
    /// Master switch. When false no session is tracked, and an untracked
    /// session admits everything.
    pub admission_enabled: bool,
    /// Invariant 1 — lifetime agent budget.
    pub agent_cap: bool,
    /// Invariant 2 — single writer per artifact.
    pub single_writer: bool,
    /// Invariant 3 — irreversible action with no passed gate dominating it.
    pub ungated_irreversible: GateEnforcementConfig,
    /// Lifetime agent ceiling for a session that declares no graph.
    ///
    /// A declared graph carries its own `GraphBudget::max_agents` and that wins.
    /// This is the number for an ordinary turn, which declares nothing.
    pub max_agents: u32,
}

impl Default for TopologyConfig {
    fn default() -> Self {
        Self {
            admission_enabled: true,
            agent_cap: true,
            single_writer: true,
            ungated_irreversible: GateEnforcementConfig::default(),
            // Mirrors `WorkflowSpec::default_max_agents` and
            // `GraphBudget::default().max_agents`, so a plain session and a
            // declared graph that says nothing agree.
            max_agents: crate::orchestrator::pool::DEFAULT_MAX_AGENTS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_invariant_defaults_to_on() {
        let config = TopologyConfig::default();
        assert!(config.admission_enabled);
        assert!(config.agent_cap);
        assert!(config.single_writer);
        assert_ne!(config.ungated_irreversible, GateEnforcementConfig::Off);
    }

    #[test]
    fn an_empty_table_round_trips_to_the_defaults() {
        let parsed: TopologyConfig = toml::from_str("").expect("empty table parses");
        assert_eq!(parsed, TopologyConfig::default());
    }

    #[test]
    fn one_invariant_can_be_turned_off_without_naming_the_others() {
        let parsed: TopologyConfig =
            toml::from_str("single_writer = false").expect("partial table parses");
        assert!(!parsed.single_writer);
        assert!(parsed.agent_cap);
        assert!(parsed.admission_enabled);
    }

    #[test]
    fn gate_enforcement_parses_by_snake_case_name() {
        let parsed: TopologyConfig =
            toml::from_str(r#"ungated_irreversible = "always""#).expect("parses");
        assert_eq!(parsed.ungated_irreversible, GateEnforcementConfig::Always);
        let parsed: TopologyConfig =
            toml::from_str(r#"ungated_irreversible = "off""#).expect("parses");
        assert_eq!(parsed.ungated_irreversible, GateEnforcementConfig::Off);
    }
}
