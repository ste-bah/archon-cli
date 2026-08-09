use super::*;

use archon_world_model::VerificationKind;

fn cognitive_workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join(".archon").join("cognitive"))
        .expect("cognitive store dir");
    temp
}

fn context(working_dir: &Path) -> LatentSurpriseContext<'_> {
    LatentSurpriseContext {
        working_dir,
        session_id: "session-1",
        turn_number: 3,
        model_id: "model-under-test",
    }
}

fn verification(status: VerificationStatus) -> VerificationOutcome {
    VerificationOutcome {
        action_id: "action-1".into(),
        kind: VerificationKind::UnitTests,
        status,
        summary: "cargo test".into(),
        idempotency_key: "world_guardrail:verification:action-1:unit_tests".into(),
        ..VerificationOutcome::default()
    }
}

fn outcome(
    surprise: Option<f32>,
    prediction_id: Option<&str>,
    verifications: Vec<VerificationOutcome>,
) -> WorldGuardrailOutcome {
    WorldGuardrailOutcome {
        outcome_id: "world-guard-outcome-1".into(),
        action_id: "action-1".into(),
        prediction_id: prediction_id.map(str::to_string),
        task_class: RuntimeTaskClass::CodingChange,
        final_status: GuardrailFinalStatus::CompletedVerified,
        verification_outcomes: verifications,
        latent_surprise: surprise,
        actual_summary: "interactive turn completed".into(),
        ..WorldGuardrailOutcome::default()
    }
}

fn snapshot(working_dir: &Path) -> archon_cognitive::CognitiveMetricSnapshot {
    let root = working_dir.join(".archon").join("cognitive");
    let store = PersistentCognitiveStore::open(&root).expect("cognitive store");
    MetricEventStore::new(store.db(), store.root())
        .expect("metric event store")
        .latest_snapshot()
        .expect("snapshot")
}

/// The whole point of the producer: `latent_surprise_mean` and `_p95` stop
/// being definitions nothing feeds. A live guarded action that carried a
/// prediction and was verified writes one event, and the derivation reads a
/// number out of it.
#[test]
fn a_verified_predicted_action_feeds_the_latent_surprise_metrics() {
    let temp = cognitive_workspace();
    let outcome = outcome(
        Some(0.75),
        Some("world-prediction-1"),
        vec![verification(VerificationStatus::Passed)],
    );

    let written = record_latent_surprise(context(temp.path()), &outcome).expect("record");

    assert!(matches!(
        written,
        Some(archon_cognitive::MetricWriteOutcome::Written)
    ));
    let snapshot = snapshot(temp.path());
    assert_eq!(
        snapshot.pooled("latent_surprise_mean").unwrap().value,
        Some(0.75)
    );
    assert_eq!(
        snapshot.pooled("latent_surprise_p95").unwrap().value,
        Some(0.75)
    );
}

/// The identities are the point. Without all three the event kind is not
/// writable, and inventing one would be exactly the fabrication this producer
/// exists to avoid.
#[test]
fn an_action_missing_any_identity_records_nothing() {
    let temp = cognitive_workspace();

    let no_prediction = outcome(
        Some(0.75),
        None,
        vec![verification(VerificationStatus::Passed)],
    );
    let no_verification = outcome(Some(0.75), Some("world-prediction-1"), Vec::new());
    let no_surprise = outcome(
        None,
        Some("world-prediction-1"),
        vec![verification(VerificationStatus::Passed)],
    );

    for outcome in [no_prediction, no_verification, no_surprise] {
        assert!(
            record_latent_surprise(context(temp.path()), &outcome)
                .expect("record")
                .is_none()
        );
    }
    assert_eq!(snapshot(temp.path()).event_count, 0);
}

/// A skipped or never-run verification adjudicates nothing. Anchoring to one
/// would claim the surprise was checked against a verified outcome when it was
/// not — the failure mode issue #153 exists for.
#[test]
fn a_verification_that_never_ran_is_not_an_anchor() {
    let temp = cognitive_workspace();

    for status in [
        VerificationStatus::Skipped,
        VerificationStatus::NotRun,
        VerificationStatus::Inconclusive,
    ] {
        let outcome = outcome(
            Some(0.75),
            Some("world-prediction-1"),
            vec![verification(status)],
        );

        assert!(
            record_latent_surprise(context(temp.path()), &outcome)
                .expect("record")
                .is_none(),
            "{status:?} was treated as an adjudication"
        );
    }
}

/// A failure decided the action's final status, so it is what the surprise is
/// measured against even when a passing verification also exists.
#[test]
fn a_failure_outranks_a_pass_when_choosing_the_anchor() {
    let temp = cognitive_workspace();
    let mut passed = verification(VerificationStatus::Passed);
    passed.idempotency_key = "world_guardrail:verification:action-1:aaa".into();
    let mut failed = verification(VerificationStatus::Failed);
    failed.idempotency_key = "world_guardrail:verification:action-1:zzz".into();
    let outcome = outcome(
        Some(0.9),
        Some("world-prediction-1"),
        vec![passed, failed.clone()],
    );

    record_latent_surprise(context(temp.path()), &outcome).expect("record");

    let root = temp.path().join(".archon").join("cognitive");
    let store = PersistentCognitiveStore::open(&root).expect("cognitive store");
    let events = MetricEventStore::new(store.db(), store.root())
        .expect("metric event store")
        .events()
        .expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].identity("verification_id"),
        Some(failed.idempotency_key.as_str())
    );
    assert_eq!(events[0].outcome_status, "failed");
}

/// Writing a measurement is no reason to create a database the session never
/// asked for.
#[test]
fn a_workspace_without_a_cognitive_store_is_left_alone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outcome = outcome(
        Some(0.75),
        Some("world-prediction-1"),
        vec![verification(VerificationStatus::Passed)],
    );

    assert!(
        record_latent_surprise(context(temp.path()), &outcome)
            .expect("record")
            .is_none()
    );
    assert!(!temp.path().join(".archon").join("cognitive").exists());
}

/// The event id is derived from the outcome, so the same observation reaching
/// the store twice is a replay rather than a second row inflating the mean.
#[test]
fn recording_the_same_outcome_twice_is_a_replay() {
    let temp = cognitive_workspace();
    let outcome = outcome(
        Some(0.75),
        Some("world-prediction-1"),
        vec![verification(VerificationStatus::Passed)],
    );

    record_latent_surprise(context(temp.path()), &outcome).expect("first record");
    let second = record_latent_surprise(context(temp.path()), &outcome).expect("second record");

    assert!(matches!(
        second,
        Some(archon_cognitive::MetricWriteOutcome::DuplicateIgnored)
    ));
    assert_eq!(snapshot(temp.path()).event_count, 1);
}
