use std::collections::BTreeMap;

use archon_world_model::guardrail::{
    GuardedActionKind, GuardrailFinalStatus, RuntimeTaskClass, VerificationKind,
    VerificationOutcome, VerificationRequirement, VerificationStatus, WorldGuardedAction,
    WorldGuardrailOutcome, append_guarded_action, append_guardrail_outcome,
    append_verification_outcome,
};
use archon_world_model::integration::WorldAdvisorSurface;
use archon_world_model::labels::DeterministicLabelBuilder;
use archon_world_model::materialize::{
    append_materialized_labels, binary_success_labels, materialize_verified_labels,
};
use archon_world_model::schema::{WorldActionKind, WorldTraceRow};
use archon_world_model::storage::WorldModelStore;

fn action(action_id: &str) -> WorldGuardedAction {
    let mut action = WorldGuardedAction::new(
        "session-1",
        WorldAdvisorSurface::ToolRun,
        GuardedActionKind::PlanStep,
        "test",
        "test",
    );
    action.action_id = action_id.into();
    action.idempotency_key = format!("world_guardrail:action:{action_id}");
    action.verification_plan = vec![VerificationRequirement {
        requirement_id: "required-tests".into(),
        kind: VerificationKind::UnitTests,
        required_for_final: true,
        ..VerificationRequirement::default()
    }];
    action
}

fn verification(action_id: &str, status: VerificationStatus) -> VerificationOutcome {
    VerificationOutcome {
        action_id: action_id.into(),
        requirement_id: "required-tests".into(),
        kind: VerificationKind::UnitTests,
        status,
        idempotency_key: format!("verification:{action_id}:{status:?}"),
        ..VerificationOutcome::default()
    }
}

fn outcome(
    action_id: &str,
    status: GuardrailFinalStatus,
    verification_status: Option<VerificationStatus>,
) -> WorldGuardrailOutcome {
    WorldGuardrailOutcome {
        outcome_id: format!("outcome-{action_id}"),
        action_id: action_id.into(),
        prediction_id: Some(format!("prediction-{action_id}")),
        task_class: RuntimeTaskClass::CodingChange,
        final_status: status,
        verification_outcomes: verification_status
            .map(|status| vec![verification(action_id, status)])
            .unwrap_or_default(),
        idempotency_key: format!("world_guardrail:outcome:{action_id}"),
        ..WorldGuardrailOutcome::default()
    }
}

fn materialize(
    rows: &[WorldTraceRow],
    actions: &[WorldGuardedAction],
    outcomes: &[WorldGuardrailOutcome],
) -> archon_world_model::materialize::MaterializedLabels {
    let verifications = outcomes
        .iter()
        .flat_map(|outcome| outcome.verification_outcomes.clone())
        .collect::<Vec<_>>();
    let predictions = outcomes
        .iter()
        .filter_map(|outcome| {
            outcome
                .prediction_id
                .as_ref()
                .map(|prediction_id| (prediction_id.clone(), outcome.action_id.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    materialize_verified_labels(rows, actions, outcomes, &verifications, &predictions).unwrap()
}

fn write_prediction(root: &std::path::Path, action_id: &str) {
    let prediction_id = format!("prediction-{action_id}");
    let directory = root.join("predictions");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{prediction_id}.json")),
        serde_json::to_vec(&serde_json::json!({
            "prediction_id": prediction_id,
            "action_ref": action_id,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn trace(action_id: &str, text: &str) -> WorldTraceRow {
    let mut row = WorldTraceRow::new("session-1", WorldActionKind::ToolCall)
        .with_row_id(format!("trace-{action_id}"))
        .with_action_attempt_id(action_id);
    row.redacted_excerpt = Some(text.into());
    row.labels = DeterministicLabelBuilder.label_row(&row);
    row
}

#[test]
fn completion_prose_never_creates_a_positive_label() {
    let labels = DeterministicLabelBuilder.label_row(&trace(
        "attempt-1",
        "completed successfully without verification",
    ));
    assert_eq!(labels.success, None);

    let labels =
        DeterministicLabelBuilder.label_row(&trace("attempt-2", "failed, then marked completed"));
    assert_eq!(labels.success, Some(false));
}

#[test]
fn materialization_uses_required_verification_precedence() {
    let actions = [
        action("passed"),
        action("failed-verification"),
        action("failed-execution"),
        action("absent"),
        action("skipped"),
        action("not-run"),
        action("inconclusive"),
        action("manual-override"),
    ];
    let rows = actions
        .iter()
        .map(|action| trace(&action.action_id, "completed successfully"))
        .collect::<Vec<_>>();
    let outcomes = [
        outcome(
            "passed",
            GuardrailFinalStatus::CompletedVerified,
            Some(VerificationStatus::Passed),
        ),
        outcome(
            "failed-verification",
            GuardrailFinalStatus::BlockedFailedVerification,
            Some(VerificationStatus::Failed),
        ),
        outcome("failed-execution", GuardrailFinalStatus::Failed, None),
        outcome("absent", GuardrailFinalStatus::CompletedWithCaveat, None),
        outcome(
            "skipped",
            GuardrailFinalStatus::CompletedWithCaveat,
            Some(VerificationStatus::Skipped),
        ),
        outcome(
            "not-run",
            GuardrailFinalStatus::CompletedWithCaveat,
            Some(VerificationStatus::NotRun),
        ),
        outcome(
            "inconclusive",
            GuardrailFinalStatus::CompletedWithCaveat,
            Some(VerificationStatus::Inconclusive),
        ),
        outcome(
            "manual-override",
            GuardrailFinalStatus::UserApprovedDespiteRisk,
            Some(VerificationStatus::Skipped),
        ),
    ];

    let materialized = materialize(&rows, &actions, &outcomes);
    let success = materialized
        .records
        .iter()
        .map(|record| (record.action_attempt_id.as_str(), record.labels.success))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(success["passed"], Some(true));
    assert_eq!(success["failed-verification"], Some(false));
    assert_eq!(success["failed-execution"], Some(false));
    for unknown in [
        "absent",
        "skipped",
        "not-run",
        "inconclusive",
        "manual-override",
    ] {
        assert_eq!(success[unknown], None, "{unknown} must remain unknown");
    }
}

#[test]
fn materialization_preserves_retry_attempt_identity_and_typed_references() {
    let actions = [action("tool-attempt-1"), action("tool-attempt-2")];
    let rows = [
        trace("tool-attempt-1", "completed"),
        trace("tool-attempt-2", "completed"),
    ];
    let outcomes = [
        outcome("tool-attempt-1", GuardrailFinalStatus::Failed, None),
        outcome(
            "tool-attempt-2",
            GuardrailFinalStatus::CompletedVerified,
            Some(VerificationStatus::Passed),
        ),
    ];

    let materialized = materialize(&rows, &actions, &outcomes);

    assert_eq!(materialized.records.len(), 2);
    assert_eq!(materialized.records[0].trace_row_id, "trace-tool-attempt-1");
    assert_eq!(
        materialized.records[0].prediction_id.as_deref(),
        Some("prediction-tool-attempt-1")
    );
    assert_eq!(materialized.records[0].verification_keys.len(), 0);
    assert_eq!(materialized.records[1].verification_keys.len(), 1);
    assert_eq!(materialized.records[0].labels.success, Some(false));
    assert_eq!(materialized.records[1].labels.success, Some(true));
}

#[test]
fn classified_correction_overrides_heuristic_correction() {
    let action = action("correction");
    let row = trace("correction", "user correction mentioned heuristically");
    assert!(row.labels.user_correction);
    let no_correction = outcome(
        "correction",
        GuardrailFinalStatus::CompletedVerified,
        Some(VerificationStatus::Passed),
    );

    let materialized = materialize(&[row], &[action], &[no_correction]);

    assert!(!materialized.records[0].labels.user_correction);
    assert_eq!(materialized.contradictions.len(), 1);
}

#[test]
fn complete_verification_ledger_prevents_conflicting_positive_label() {
    let action = action("attempt-1");
    let row = trace("attempt-1", "completed");
    let passed = outcome(
        "attempt-1",
        GuardrailFinalStatus::CompletedVerified,
        Some(VerificationStatus::Passed),
    );
    let mut persisted = passed.verification_outcomes.clone();
    persisted.push(verification("attempt-1", VerificationStatus::Skipped));
    let predictions =
        BTreeMap::from([("prediction-attempt-1".to_string(), "attempt-1".to_string())]);

    let materialized =
        materialize_verified_labels(&[row], &[action], &[passed], &persisted, &predictions)
            .unwrap();

    assert_eq!(materialized.records[0].labels.success, None);
}

#[test]
fn embedded_verification_requires_matching_persisted_record() {
    let action = action("attempt-1");
    let row = trace("attempt-1", "completed");
    let passed = outcome(
        "attempt-1",
        GuardrailFinalStatus::CompletedVerified,
        Some(VerificationStatus::Passed),
    );
    let predictions =
        BTreeMap::from([("prediction-attempt-1".to_string(), "attempt-1".to_string())]);

    let error =
        materialize_verified_labels(&[row], &[action], &[passed], &[], &predictions).unwrap_err();

    assert!(error.to_string().contains("missing persisted verification"));
}

#[test]
fn prediction_reference_must_match_action_attempt() {
    let action = action("attempt-1");
    let row = trace("attempt-1", "completed");
    let passed = outcome(
        "attempt-1",
        GuardrailFinalStatus::CompletedVerified,
        Some(VerificationStatus::Passed),
    );
    let persisted = passed.verification_outcomes.clone();
    let predictions = BTreeMap::from([(
        "prediction-attempt-1".to_string(),
        "different-attempt".to_string(),
    )]);

    let error = materialize_verified_labels(&[row], &[action], &[passed], &persisted, &predictions)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("references action different-attempt")
    );
}

#[test]
fn materialized_label_ledger_is_idempotent_and_rejects_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let action = action("attempt-1");
    let row = trace("attempt-1", "completed");
    let passed = outcome(
        "attempt-1",
        GuardrailFinalStatus::CompletedVerified,
        Some(VerificationStatus::Passed),
    );
    let materialized = materialize(&[row], &[action], &[passed]);

    let path = append_materialized_labels(temp.path(), &materialized.records).unwrap();
    append_materialized_labels(temp.path(), &materialized.records).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);

    let mut conflicting = materialized.records[0].clone();
    conflicting.labels.success = Some(false);
    let error = append_materialized_labels(temp.path(), &[conflicting]).unwrap_err();
    assert!(error.to_string().contains("conflicting materialized label"));
}

#[test]
fn binary_success_evaluation_excludes_unknown_and_reports_coverage() {
    let actions = [action("passed"), action("failed"), action("unknown")];
    let rows = [
        trace("passed", "completed"),
        trace("failed", "failed"),
        trace("unknown", "completed"),
    ];
    let outcomes = [
        outcome(
            "passed",
            GuardrailFinalStatus::CompletedVerified,
            Some(VerificationStatus::Passed),
        ),
        outcome("failed", GuardrailFinalStatus::Failed, None),
        outcome("unknown", GuardrailFinalStatus::CompletedWithCaveat, None),
    ];
    let materialized = materialize(&rows, &actions, &outcomes);

    let binary = binary_success_labels(&materialized.records);

    assert_eq!(binary.labels.len(), 2);
    assert_eq!(binary.known, 2);
    assert_eq!(binary.unknown, 1);
    assert_eq!(binary.total, 3);
}

#[test]
fn store_loads_training_rows_with_guardrail_labels_joined_separately() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorldModelStore::open(temp.path()).unwrap();
    let action = action("attempt-1");
    let row = trace("attempt-1", "completed successfully");
    let mut unjoined = trace("unjoined", "completed successfully");
    unjoined.action_attempt_id = None;
    unjoined.labels.success = Some(true);
    store.persist_rows(&[row, unjoined]).unwrap();
    append_guarded_action(temp.path(), &action).unwrap();
    let stored_outcome = outcome(
        "attempt-1",
        GuardrailFinalStatus::CompletedVerified,
        Some(VerificationStatus::Passed),
    );
    write_prediction(temp.path(), "attempt-1");
    append_verification_outcome(temp.path(), &stored_outcome.verification_outcomes[0]).unwrap();
    append_guardrail_outcome(temp.path(), &stored_outcome).unwrap();

    let rows = store.load_verified_training_rows().unwrap();

    assert_eq!(rows.len(), 2);
    let success = rows
        .iter()
        .map(|row| (row.row_id.as_str(), row.labels.success))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(success["trace-attempt-1"], Some(true));
    assert_eq!(success["trace-unjoined"], None);
}
