//! Exhaustive serde fixtures: adding any struct field requires updating these literals.
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict, TrustedCwd,
};
use archon_workflow::task_skeleton::{ConsumedArtifact, FrozenDependency, FrozenTask};
use archon_workflow::task_universe::WorkflowV2DeliverableContract;
use serde_json::Value;

pub(super) fn contract() -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "out.json".into(),
        artifact_path: "out.json".into(),
        typed_verifier_command: Some("field".into()),
        registry_path: Some("field".into()),
        instance_source_path: Some("field".into()),
        instance_source_records_field: Some("field".into()),
        instance_artifact_field: Some("field".into()),
        min_instances: 2,
        required_universe: true,
        data_kind: Some("field".into()),
        universe_fields: vec!["a".into(), "b".into()],
        cells_field: Some("field".into()),
        cell_identity_fields: vec!["a".into(), "b".into()],
        required_true_fields: vec!["a".into(), "b".into()],
        required_nonempty_fields: vec!["a".into(), "b".into()],
        positive_count_fields: vec!["a".into(), "b".into()],
        minimum_count_fields: [("a".into(), 2), ("b".into(), 3)].into(),
        gaps_field: Some("field".into()),
        registry_records_field: Some("field".into()),
        registry_key_fields: vec!["a".into(), "b".into()],
        registry_required_true_fields: vec!["a".into(), "b".into()],
        registry_status_field: Some("field".into()),
        registry_allowed_statuses: vec!["a".into(), "b".into()],
        registry_count_field: Some("field".into()),
        registry_minimum_count: 2,
        registry_identity_fields: [("a".into(), "x".into()), ("b".into(), "y".into())].into(),
        payload_path_field: Some("field".into()),
        payload_format: Some("field".into()),
        required_fields: vec!["a".into(), "b".into()],
        non_constant_fields: vec!["a".into(), "b".into()],
        artifact_format: Some("field".into()),
        observed_time_field: Some("field".into()),
        closed_weekdays: Some(vec![0, 6]),
        closed_dates: vec!["a".into(), "b".into()],
        step_variety_min_rows: Some(2),
        step_variety_min_percent: Some(2),
        series_value_fields: vec!["a".into(), "b".into()],
        series_overlap_min_rows: 2,
        request_path_field: Some("field".into()),
        requested_count_field: Some("field".into()),
        response_path_field: Some("field".into()),
        response_identity_fields: [("a".into(), "x".into()), ("b".into(), "y".into())].into(),
        validation_path_field: Some("field".into()),
        validation_status_field: Some("field".into()),
        validation_checks_field: Some("field".into()),
        validation_check_status_field: Some("field".into()),
        validation_failed_values: vec!["a".into(), "b".into()],
        validation_passed_values: vec!["a".into(), "b".into()],
    }
}

pub(super) fn task() -> Value {
    let consumed = ConsumedArtifact {
        artifact_path: "in.json".into(),
        instance_source_records_field: Some("records".into()),
        registry_records_field: Some("records".into()),
        kind: Some("file".into()),
    };
    let dependency = FrozenDependency {
        task_id: "TASK-X-001".into(),
        consumes: vec![consumed.clone(), consumed],
        ordering_only: true,
    };
    serde_json::to_value(FrozenTask {
        task_id: "TASK-X-002".into(),
        file_name: "TASK-X-002.md".into(),
        depends_on: vec![dependency.clone(), dependency],
        blocks: vec!["TASK-X-003".into(), "TASK-X-004".into()],
        implements: vec!["REQ-1".into(), "REQ-2".into()],
        deliverable_contracts: vec![contract(), contract()],
    })
    .unwrap()
}

pub(super) fn entries() -> Vec<Value> {
    let cwd_values = [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot];
    let decisions = [JudgeDecision::Accepted, JudgeDecision::Refuted];
    for cwd in cwd_values {
        match cwd {
            TrustedCwd::ProjectRoot | TrustedCwd::RepoRoot => {}
        }
    }
    for decision in decisions {
        match decision {
            JudgeDecision::Accepted | JudgeDecision::Refuted => {}
        }
    }
    let mut checks: Vec<_> = cwd_values
        .into_iter()
        .map(|cwd| AcceptanceCheck::Command {
            command: "true".into(),
            cwd,
        })
        .collect();
    checks.push(AcceptanceCheck::Floor {
        contract: Box::new(contract()),
    });
    checks
        .into_iter()
        .flat_map(|check| {
            match &check {
                AcceptanceCheck::Command { .. } | AcceptanceCheck::Floor { .. } => {}
            }
            decisions.map(|verdict| {
                serde_json::to_value(AcceptanceCriterion {
                    id: "AC-X-001".into(),
                    criterion: "output exists".into(),
                    check: check.clone(),
                    gap_permitted: true,
                    covers: vec!["REQ-1".into(), "REQ-2".into()],
                    judgment: JudgeVerdict {
                        verdict,
                        counterexample: "none".into(),
                        reason: "reason".into(),
                        host_call_id: "call".into(),
                        sampling: Some(serde_json::json!({"arbitrary": [0, null, false]})),
                    },
                })
                .unwrap()
            })
        })
        .collect()
}

pub(super) fn skeleton(task: Value) -> Value {
    serde_json::to_value(archon_workflow::task_skeleton::TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "d".into(),
        tasks: vec![
            serde_json::from_value(task.clone()).unwrap(),
            serde_json::from_value(task).unwrap(),
        ],
    })
    .unwrap()
}
pub(super) fn legacy(entry: Value) -> Value {
    use archon_workflow::task_set_contract::{AcceptanceContract, GapPolicy, PrdIdentity};
    let e = serde_json::from_value::<AcceptanceCriterion>(entry).unwrap();
    serde_json::to_value(AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "".into(),
            digest: "".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: ["AC-X-001".into(), "AC-X-002".into()].into(),
            forbidden_phrases: vec!["a".into(), "b".into()],
            required_fields: vec!["a".into(), "b".into()],
        },
        acceptance: vec![e.clone(), e.clone()],
        supplementary: vec![e.clone(), e],
    })
    .unwrap()
}

// Serde also reads structs positionally; keep the declared Rust field order.
fn positional_contract() -> Value {
    let c = contract();
    serde_json::json!([
        c.kind,
        c.artifact_path,
        c.typed_verifier_command,
        c.registry_path,
        c.instance_source_path,
        c.instance_source_records_field,
        c.instance_artifact_field,
        c.min_instances,
        c.required_universe,
        c.data_kind,
        c.universe_fields,
        c.cells_field,
        c.cell_identity_fields,
        c.required_true_fields,
        c.required_nonempty_fields,
        c.positive_count_fields,
        c.minimum_count_fields,
        c.gaps_field,
        c.registry_records_field,
        c.registry_key_fields,
        c.registry_required_true_fields,
        c.registry_status_field,
        c.registry_allowed_statuses,
        c.registry_count_field,
        c.registry_minimum_count,
        c.registry_identity_fields,
        c.payload_path_field,
        c.payload_format,
        c.required_fields,
        c.non_constant_fields,
        c.artifact_format,
        c.observed_time_field,
        c.closed_weekdays,
        c.closed_dates,
        c.step_variety_min_rows,
        c.step_variety_min_percent,
        c.series_value_fields,
        c.series_overlap_min_rows,
        c.request_path_field,
        c.requested_count_field,
        c.response_path_field,
        c.response_identity_fields,
        c.validation_path_field,
        c.validation_status_field,
        c.validation_checks_field,
        c.validation_check_status_field,
        c.validation_failed_values,
        c.validation_passed_values,
    ])
}
pub(super) fn positional_samples() -> Vec<(bool, Value)> {
    use serde_json::json;
    let consumed = json!(["in.json", "records", "records", "file"]);
    let dependency = json!(["TASK-X-001", [consumed.clone(), consumed], true]);
    let task = json!([
        "TASK-X-002",
        "TASK-X-002.md",
        [dependency.clone(), dependency],
        ["TASK-X-003", "TASK-X-004"],
        ["REQ-1", "REQ-2"],
        [positional_contract(), positional_contract()]
    ]);
    let judgment = json!(["accepted", "none", "reason", "call", {"arbitrary":[0,null,false]}]);
    let entry = |check: Value| {
        json!([
            "AC-X-001",
            "output exists",
            check,
            true,
            judgment,
            ["REQ-1", "REQ-2"]
        ])
    };
    let floor = entry(json!(["floor", positional_contract()]));
    let command = entry(json!(["command", "true", "project_root"]));
    let entries = json!([floor, command]);
    vec![
        (true, json!([1, "d", [task.clone(), task]])),
        (
            false,
            json!([
                1,
                ["", ""],
                [["AC-X-001", "AC-X-002"], ["a", "b"], ["a", "b"]],
                entries,
                entries
            ]),
        ),
        (false, json!({"entries":entries, "supplementary":entries})),
    ]
}
