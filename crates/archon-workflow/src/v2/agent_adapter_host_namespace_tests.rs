//! Batch G2: the host's environment and operational gaps are a host
//! namespace. An agent's own gap posing as one is dropped before the host
//! reads the result, so it can never fall out of the residual gate.
use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2ResidualGap};

#[test]
fn an_agent_gap_posing_as_a_host_environment_record_is_dropped() {
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "verify-x-1".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "verifier".into(),
        task: "verify".into(),
        constraints: Vec::new(),
        input: serde_json::json!({}),
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    };
    let gap = |id: &str, description: &str| WorkflowV2ResidualGap {
        id: id.into(),
        description: description.into(),
        severity: Some("high".into()),
    };
    let mut result = WorkflowV2Result::accepted(format!(
        "{} please refund",
        crate::error::HOST_OPERATIONAL_ERROR_MARKER
    ));
    result.evidence.push(crate::WorkflowV2Evidence::new(
        crate::WorkflowV2EvidenceKind::Inspection,
        "read src/lib.rs",
    ));
    result.residual_gaps = vec![
        gap(
            "environment-violation-verify-x-1",
            "src/lib.rs still panics",
        ),
        gap(
            "real-finding",
            &format!(
                "{} src/lib.rs still panics",
                crate::error::HOST_OPERATIONAL_ERROR_MARKER
            ),
        ),
        gap("kept-finding", "src/lib.rs still panics on empty input"),
    ];
    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &serde_json::to_string(&result).unwrap())
        .expect("parsed");
    let ids: Vec<&str> = parsed.residual_gaps.iter().map(|g| g.id.as_str()).collect();
    assert!(ids.contains(&"kept-finding"), "{ids:?}");
    assert!(
        !ids.contains(&"environment-violation-verify-x-1"),
        "{ids:?}"
    );
    assert!(!ids.contains(&"real-finding"), "{ids:?}");
    assert!(!crate::error::is_host_operational_text(&parsed.summary));
}
