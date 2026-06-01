use super::{AgentInfo, PIPELINE_MAX_ATTEMPTS, QualityScore};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineRunOptions {
    pub force_quality_gate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct QualityGateAcceptance {
    pub accepted: bool,
    pub force_accepted: bool,
}

pub(super) fn quality_gate_acceptance(
    meets_threshold: bool,
    agent: &AgentInfo,
    attempt: usize,
    options: PipelineRunOptions,
) -> QualityGateAcceptance {
    let natural_accept = attempt_accepted(meets_threshold, agent.critical, attempt);
    let final_critical_miss =
        !meets_threshold && agent.critical && attempt >= PIPELINE_MAX_ATTEMPTS;
    let force_accepted = options.force_quality_gate && final_critical_miss;
    QualityGateAcceptance {
        accepted: natural_accept || force_accepted,
        force_accepted,
    }
}

pub(super) fn force_acceptance_reason(base_reason: Option<String>) -> String {
    match base_reason {
        Some(reason) => format!("force-accepted below quality threshold: {reason}"),
        None => "force-accepted below quality threshold".to_string(),
    }
}

pub(super) fn has_non_bypassable_quality_failure(quality: &QualityScore) -> bool {
    quality.overall == 0.0 && quality.dimensions.contains_key("citation_gate")
}

pub(super) fn attempt_accepted(meets_threshold: bool, critical: bool, attempt: usize) -> bool {
    meets_threshold || (!critical && attempt >= PIPELINE_MAX_ATTEMPTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::ToolAccessLevel;
    use std::collections::HashMap;

    fn critical_agent() -> AgentInfo {
        AgentInfo {
            key: "introduction-writer".into(),
            display_name: "Introduction Writer".into(),
            model: "sonnet".into(),
            phase: 6,
            critical: true,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access_level: ToolAccessLevel::ReadOnly,
        }
    }

    #[test]
    fn force_quality_gate_accepts_final_critical_miss_only_when_enabled() {
        let agent = critical_agent();
        let blocked =
            quality_gate_acceptance(false, &agent, PIPELINE_MAX_ATTEMPTS, Default::default());
        assert!(!blocked.accepted);
        assert!(!blocked.force_accepted);

        let options = PipelineRunOptions {
            force_quality_gate: true,
        };
        let forced = quality_gate_acceptance(false, &agent, PIPELINE_MAX_ATTEMPTS, options);
        assert!(forced.accepted);
        assert!(forced.force_accepted);
    }

    #[test]
    fn force_quality_gate_does_not_short_circuit_early_attempts() {
        let agent = critical_agent();
        let options = PipelineRunOptions {
            force_quality_gate: true,
        };
        let decision = quality_gate_acceptance(false, &agent, 1, options);
        assert!(!decision.accepted);
        assert!(!decision.force_accepted);
    }

    #[test]
    fn semantic_citation_gate_failure_is_non_bypassable() {
        let quality = QualityScore {
            overall: 0.0,
            dimensions: HashMap::from([("citation_gate".to_string(), 0.0)]),
        };
        assert!(has_non_bypassable_quality_failure(&quality));
    }
}
