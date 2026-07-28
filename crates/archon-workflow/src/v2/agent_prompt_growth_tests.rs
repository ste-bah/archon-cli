use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "call-1".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "Implement TASK-1".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

#[test]
fn reducer_prompt_digests_positional_review_evidence() {
    let mut request = request();
    request.call.id = "adversarial-review-3".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "source_data": [[
            review_record(1, "review-old-detail"),
            review_record(2, "review-latest-detail")
        ]]
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(!prompt.invocation.contains("review-old-detail"));
    assert!(prompt.invocation.contains("review-latest-detail"));
}

#[test]
fn reducer_prompt_digests_positional_lifecycle_evidence() {
    let mut request = request();
    request.call.id = "remediation-inventory-4".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "source_data": [
            {"inventory":"current"},
            [
                evidence_record(1, "positional-old-detail"),
                evidence_record(2, "positional-latest-detail")
            ]
        ]
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(!prompt.invocation.contains("positional-old-detail"));
    assert!(prompt.invocation.contains("positional-latest-detail"));
}

#[test]
fn final_report_prompt_digests_old_lifecycle_evidence() {
    let mut request = request();
    request.call.id = "blocked-remediation-unresolved-4".into();
    request.call.method = WorkflowV2HostMethod::FinalReport;
    request.input = serde_json::json!({
        "inputs": {
            "implementationEvidence": [
                evidence_record(1, "final-old-detail"),
                evidence_record(2, "final-latest-detail")
            ]
        }
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(!prompt.invocation.contains("final-old-detail"));
    assert!(prompt.invocation.contains("final-latest-detail"));
}

#[test]
fn reducer_prompt_keeps_latest_wave_full_and_digests_older_waves() {
    let mut request = request();
    request.call.id = "completion-evidence-repair-4".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "implementationEvidence": [
            evidence_record(1, "old-private-detail"),
            evidence_record(2, "middle-private-detail"),
            evidence_record(3, "latest-full-detail")
        ]
    });
    let original = request.input.clone();

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(!prompt.invocation.contains("old-private-detail"));
    assert!(!prompt.invocation.contains("middle-private-detail"));
    assert!(prompt.invocation.contains("latest-full-detail"));
    assert!(prompt.invocation.contains("implementationWaveIndex"));
    assert_eq!(
        request.input, original,
        "prompt projection must not mutate persisted input"
    );
}

#[test]
fn positional_evidence_with_real_resultless_markers_still_digests_old_results() {
    for marker in [
        serde_json::json!({
            "kind":"implementation-ownership-discovery-pending",
            "canonical_task_ids":["TASK-0"]
        }),
        serde_json::json!({
            "kind":"required-artifacts",
            "requiredArtifacts":{"status":"accepted"}
        }),
    ] {
        let mut request = request();
        request.call.id = "final-evidence-reconciliation-4".into();
        request.call.method = WorkflowV2HostMethod::Reduce;
        request.input = serde_json::json!({
            "source_data": [[
                marker,
                evidence_record(1, "structural-marker-old-detail"),
                evidence_record(2, "structural-marker-latest-detail")
            ]]
        });

        let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

        assert!(
            prompt
                .invocation
                .contains("structural-marker-latest-detail")
        );
        assert!(!prompt.invocation.contains("structural-marker-old-detail"));
    }
}

#[test]
fn older_result_collections_are_bounded_but_latest_stays_full() {
    let mut request = request();
    request.call.id = "completion-evidence-repair-2".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "implementationEvidence": [
            rich_evidence_record(1, 40),
            rich_evidence_record(2, 2)
        ]
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains(r#""outcomes_count":40"#));
    assert!(prompt.invocation.contains(r#""task_coverage_count":40"#));
    assert!(prompt.invocation.contains(r#""residual_gaps_count":40"#));
    assert!(!prompt.invocation.contains("old-outcome-summary-1-20"));
    assert!(!prompt.invocation.contains("old-coverage-summary-1-20"));
    assert!(!prompt.invocation.contains("old-gap-description-1-20"));
    assert!(prompt.invocation.contains("old-outcome-summary-1-39"));
    assert!(prompt.invocation.contains("latest-outcome-detail-2-1"));
}

#[test]
fn artifact_evidence_keeps_latest_investigation_full_and_digests_older_results() {
    let mut request = request();
    request.call.id = "final-evidence-reconciliation-3".into();
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({
        "artifactEvidence": [
            {"kind":"artifact-inventory","artifactInventory":{"status":"accepted"}},
            artifact_record(1, "artifact-old-detail"),
            artifact_record(2, "artifact-middle-detail"),
            artifact_record(3, "artifact-latest-detail")
        ]
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains("artifact-inventory"));
    assert!(!prompt.invocation.contains("artifact-old-detail"));
    assert!(!prompt.invocation.contains("artifact-middle-detail"));
    assert!(prompt.invocation.contains("artifact-latest-detail"));
}

#[test]
fn cumulative_result_collection_growth_stays_linear() {
    let projected_4 = projected_rich_growth_bytes(4);
    let projected_8 = projected_rich_growth_bytes(8);
    let raw_4 = raw_rich_growth_bytes(4);
    let raw_8 = raw_rich_growth_bytes(8);

    assert!(
        projected_8 < projected_4 * 3,
        "bounded projected collections must keep growth linear"
    );
    assert!(
        raw_8 > raw_4 * 3,
        "fixture must demonstrate superlinear cumulative collections"
    );
}

#[test]
fn wave_evidence_prompt_growth_is_linear_when_full_records_grow() {
    let projected_4 = projected_wave_bytes(4);
    let projected_8 = projected_wave_bytes(8);
    let raw_4 = raw_wave_bytes(4);
    let raw_8 = raw_wave_bytes(8);

    assert!(
        projected_8 < projected_4 * 3,
        "projected growth must stay linear"
    );
    assert!(
        raw_8 > raw_4 * 3,
        "fixture must demonstrate superlinear raw growth"
    );
}

#[test]
fn implementation_verification_and_review_growth_stays_linear() {
    for family in [
        "implementationEvidence",
        "verificationEvidence",
        "reviewEvidence",
    ] {
        let projected_4 = projected_family_bytes(family, 4);
        let projected_8 = projected_family_bytes(family, 8);
        assert!(
            projected_8 < projected_4 * 3,
            "{family} projected growth must stay linear"
        );
    }
}

fn projected_family_bytes(family: &str, count: usize) -> usize {
    let records = if family == "reviewEvidence" {
        (1..=count)
            .map(|index| review_record(index, &"x".repeat(index * 2_000)))
            .collect()
    } else {
        growing_evidence_records(count)
    };
    let mut request = request();
    request.call.id = format!("adversarial-review-{count}");
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({family:records});
    WorkflowV2AgentAdapter::new()
        .build_prompt_parts(&request)
        .invocation
        .len()
}

fn review_record(iteration: usize, detail: &str) -> serde_json::Value {
    serde_json::json!({
        "kind":"review",
        "reviewIteration":iteration,
        "result":{
            "status":"accepted",
            "summary":format!("review {iteration} accepted"),
            "outcomes":[{"item_id":format!("review-{iteration}"),"status":"accepted","detail":detail}]
        }
    })
}

fn evidence_record(wave: usize, detail: &str) -> serde_json::Value {
    serde_json::json!({
        "kind":"implementation",
        "implementationWaveIndex":wave,
        "dependencyIteration":wave,
        "readyImplementationItems":[{"item_id":format!("item-{wave}"),"detail":detail}],
        "result":{
            "status":"accepted",
            "summary":format!("wave {wave} accepted"),
            "outcomes":[{"item_id":format!("item-{wave}"),"status":"accepted","detail":detail}]
        }
    })
}

fn artifact_record(iteration: usize, detail: &str) -> serde_json::Value {
    serde_json::json!({
        "kind":"artifact-existence-investigation",
        "finalEvidenceIteration":iteration,
        "artifactChecks":[{"path":format!("artifact-{iteration}.json")}],
        "result":{
            "status":"accepted",
            "summary":format!("artifact iteration {iteration} accepted"),
            "outcomes":[{"item_id":format!("artifact-{iteration}"),"status":"accepted","detail":detail}]
        }
    })
}

fn rich_evidence_record(wave: usize, count: usize) -> serde_json::Value {
    let prefix = if wave == 1 { "old" } else { "latest" };
    let outcomes = (0..count)
        .map(|index| {
            serde_json::json!({
                "item_id":format!("item-{wave}-{index}"),
                "status":"accepted",
                "summary":format!("{prefix}-outcome-summary-{wave}-{index}"),
                "detail":format!("{prefix}-outcome-detail-{wave}-{index}")
            })
        })
        .collect::<Vec<_>>();
    let task_coverage = (0..count)
        .map(|index| {
            serde_json::json!({
                "task_id":format!("TASK-{wave}-{index}"),
                "status":"accepted",
                "summary":format!("{prefix}-coverage-summary-{wave}-{index}")
            })
        })
        .collect::<Vec<_>>();
    let residual_gaps = (0..count)
        .map(|index| {
            serde_json::json!({
                "id":format!("gap-{wave}-{index}"),
                "severity":"non_blocking",
                "description":format!("{prefix}-gap-description-{wave}-{index}")
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "kind":"implementation",
        "implementationWaveIndex":wave,
        "result":{
            "status":"accepted",
            "summary":format!("wave {wave} accepted"),
            "outcomes":outcomes,
            "task_coverage":task_coverage,
            "residual_gaps":residual_gaps
        }
    })
}

fn rich_growth_records(count: usize) -> Vec<serde_json::Value> {
    (1..=count)
        .map(|wave| rich_evidence_record(wave, wave * 10))
        .collect()
}

fn projected_rich_growth_bytes(count: usize) -> usize {
    let mut request = request();
    request.call.id = format!("completion-evidence-repair-{count}");
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({"implementationEvidence":rich_growth_records(count)});
    WorkflowV2AgentAdapter::new()
        .build_prompt_parts(&request)
        .invocation
        .len()
}

fn raw_rich_growth_bytes(count: usize) -> usize {
    serde_json::to_vec(&rich_growth_records(count))
        .expect("raw rich evidence json")
        .len()
}

fn growing_evidence_records(count: usize) -> Vec<serde_json::Value> {
    (1..=count)
        .map(|wave| evidence_record(wave, &"x".repeat(wave * 2_000)))
        .collect()
}

fn projected_wave_bytes(count: usize) -> usize {
    let mut request = request();
    request.call.id = format!("completion-evidence-repair-{count}");
    request.call.method = WorkflowV2HostMethod::Reduce;
    request.input = serde_json::json!({"implementationEvidence":growing_evidence_records(count)});
    WorkflowV2AgentAdapter::new()
        .build_prompt_parts(&request)
        .invocation
        .len()
}

fn raw_wave_bytes(count: usize) -> usize {
    serde_json::to_vec(&growing_evidence_records(count))
        .expect("raw evidence json")
        .len()
}
