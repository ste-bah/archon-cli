use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProofConfig {
    pub max_runs_per_attempt: u64,
}
impl Default for ProofConfig {
    fn default() -> Self {
        Self {
            max_runs_per_attempt: 200,
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn proof_budget_defaults_disables_and_rejects_incomplete_control_groups() {
        let mut config = crate::config::ArchonConfig::default();
        assert_eq!(config.workflow.proof.max_runs_per_attempt, 200);
        for max in [0, 3, 200] {
            config.workflow.proof.max_runs_per_attempt = max;
            crate::config::validate(&config).unwrap();
        }
        for max in [1, 2] {
            config.workflow.proof.max_runs_per_attempt = max;
            assert!(
                crate::config::validate(&config)
                    .unwrap_err()
                    .to_string()
                    .contains("at least 3")
            );
        }
    }
}
