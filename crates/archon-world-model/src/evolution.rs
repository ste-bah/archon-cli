//! World-model evidence for governed agent evolution.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::WorldTraceRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldModelEvolutionSignal {
    pub agent_type: String,
    pub signal_kind: String,
    pub evidence_ids: Vec<String>,
    pub observation_count: usize,
    pub requires_shadow_evaluation: bool,
    pub requires_approval: bool,
}

pub fn repeated_risk_signals(
    rows: &[WorldTraceRow],
    min_observations: usize,
) -> Vec<WorldModelEvolutionSignal> {
    let mut grouped: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for row in rows {
        let Some(agent) = row.agent.clone() else {
            continue;
        };
        for signal in row_signals(row) {
            grouped
                .entry((agent.clone(), signal))
                .or_default()
                .push(row.row_id.clone());
        }
    }

    grouped
        .into_iter()
        .filter(|(_, evidence_ids)| evidence_ids.len() >= min_observations)
        .map(
            |((agent_type, signal_kind), evidence_ids)| WorldModelEvolutionSignal {
                agent_type,
                signal_kind,
                observation_count: evidence_ids.len(),
                evidence_ids,
                requires_shadow_evaluation: true,
                requires_approval: true,
            },
        )
        .collect()
}

fn row_signals(row: &WorldTraceRow) -> Vec<String> {
    let mut signals = Vec::new();
    if row.labels.failure {
        signals.push("failure".into());
    }
    if row.labels.retry {
        signals.push("retry".into());
    }
    if row.labels.provider_incident {
        signals.push("provider_incident".into());
    }
    if row.labels.plan_drift {
        signals.push("plan_drift".into());
    }
    if row.labels.user_correction {
        signals.push("user_correction".into());
    }
    signals
}

#[cfg(test)]
mod tests {
    use crate::schema::{WorldActionKind, WorldTraceRow};

    use super::*;

    fn failed_row(id: &str) -> WorldTraceRow {
        let mut row = WorldTraceRow::new("s1", WorldActionKind::AgentAttempt).with_row_id(id);
        row.agent = Some("coder".into());
        row.labels.failure = true;
        row
    }

    #[test]
    fn repeated_world_model_risks_create_governed_signal() {
        let rows = [failed_row("r1"), failed_row("r2"), failed_row("r3")];

        let signals = repeated_risk_signals(&rows, 3);

        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].agent_type, "coder");
        assert_eq!(signals[0].signal_kind, "failure");
        assert!(signals[0].requires_shadow_evaluation);
        assert!(signals[0].requires_approval);
    }

    #[test]
    fn isolated_world_model_risks_do_not_create_signal() {
        let rows = [failed_row("r1")];

        let signals = repeated_risk_signals(&rows, 3);

        assert!(signals.is_empty());
    }
}
