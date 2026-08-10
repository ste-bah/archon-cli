//! Metric events for the governed proposal lifecycle, read back from the store.
//!
//! Derivation is exercised here as well as emission. An event that writes but
//! derives to nothing is the same shape of failure as one that never writes:
//! the gate sees `NotEvaluated` either way.

use archon_cognitive::PersistentCognitiveStore;
use archon_cognitive::metrics::derive::derive_snapshot;
use archon_cognitive::metrics::event_store::MetricEventStore;
use archon_cognitive::metrics::window::MetricCohort;
use archon_learning::garden_proposals::{
    GardenProposalKind, GardenProposalRecord, GardenProposalStatus,
};

use super::{
    GardenMetricContext, record_proposal_applied, record_proposal_decided, record_proposal_raised,
    record_proposal_rolled_back,
};

fn context(dir: &std::path::Path) -> GardenMetricContext {
    GardenMetricContext {
        working_dir: dir.to_path_buf(),
        model_id: "test-model".to_string(),
        session_id: "session-1".to_string(),
        turn_number: 1,
    }
}

fn proposal(kind: GardenProposalKind, subject: &str) -> GardenProposalRecord {
    GardenProposalRecord {
        proposal_id: GardenProposalRecord::stable_id(kind, subject),
        proposal_kind: kind,
        subject_id: subject.to_string(),
        subject_title: "a title".to_string(),
        excerpt: "an excerpt".to_string(),
        detail: "the evidence".to_string(),
        payload_json: "{}".to_string(),
        run_id: "run-1".to_string(),
        status: GardenProposalStatus::Pending,
        applied_ref: String::new(),
        created_at: "2026-08-10T03:00:00Z".to_string(),
        decided_at: String::new(),
    }
}

/// Every event the emitters wrote, read back from the cognitive store.
fn recorded(dir: &std::path::Path) -> Vec<archon_cognitive::metrics::event::CognitiveMetricEvent> {
    let root = dir.join(".archon").join("cognitive");
    let store = PersistentCognitiveStore::open(&root).expect("open store");
    MetricEventStore::new(store.db(), &root)
        .expect("event store")
        .events()
        .expect("read events")
}

#[test]
fn the_lifecycle_writes_one_event_per_step() {
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");

    record_proposal_raised(&context, &record);
    record_proposal_decided(&context, &record, true);
    record_proposal_applied(&context, &record, "mem-1");
    record_proposal_rolled_back(&context, &record);

    let events = recorded(dir.path());
    let operations: Vec<&str> = events
        .iter()
        .filter_map(|event| event.identity("proposal_lifecycle_operation"))
        .collect();
    for expected in ["raise", "decide", "apply", "rollback"] {
        assert!(
            operations.contains(&expected),
            "no event recorded for {expected}: {operations:?}"
        );
    }
}

#[test]
fn acceptance_derives_from_decisions_only() {
    // The population is decisions. If raise or apply events leaked into it, the
    // rate would be diluted by steps nobody made a judgement at.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());

    record_proposal_decided(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "a"),
        true,
    );
    record_proposal_decided(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "b"),
        true,
    );
    record_proposal_decided(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "c"),
        false,
    );
    record_proposal_raised(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "d"),
    );

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    let acceptance = snapshot
        .pooled("governed_proposal_acceptance_rate")
        .expect("acceptance derived");

    assert_eq!(acceptance.sample_count, 3, "only decisions are eligible");
    assert!(
        (acceptance.value.expect("value") - 2.0 / 3.0).abs() < 1e-9,
        "two of three accepted, got {:?}",
        acceptance.value
    );
}

#[test]
fn reversal_rate_is_rollbacks_over_applications() {
    // The metric that cannot be an identity rate: an application does not know
    // at write time whether it will later be undone.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());

    for subject in ["a", "b", "c", "d"] {
        let record = proposal(GardenProposalKind::MemoryRetirement, subject);
        record_proposal_applied(&context, &record, subject);
    }
    record_proposal_rolled_back(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "a"),
    );

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    let reversal = snapshot
        .pooled("governed_proposal_reversal_rate")
        .expect("reversal derived");

    assert_eq!(reversal.denominator, 4.0, "four applications");
    assert_eq!(reversal.numerator, 1.0, "one reversal");
    assert!((reversal.value.expect("value") - 0.25).abs() < 1e-9);
}

#[test]
fn decisions_are_excluded_from_the_reversal_population() {
    // Decisions carry neither numerator nor denominator. Excluding them by
    // identity rather than relying on them summing to zero means a later
    // emitter that does set a value cannot silently corrupt the ratio.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());
    let record = proposal(GardenProposalKind::MemoryRetirement, "a");

    record_proposal_decided(&context, &record, true);
    record_proposal_applied(&context, &record, "a");

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    let reversal = snapshot
        .pooled("governed_proposal_reversal_rate")
        .expect("reversal derived");

    assert_eq!(
        reversal.sample_count, 1,
        "only the application is in the reversal population"
    );
}

#[test]
fn retiring_a_rule_records_rule_churn() {
    // `rule_retire_count` had a definition and no writer. This is the writer.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());
    let record = proposal(GardenProposalKind::RuleRetirement, "rule-1");

    record_proposal_applied(&context, &record, "rule-1");

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    let retires = snapshot.pooled("rule_retire_count").expect("churn derived");
    assert_eq!(retires.value, Some(1.0));
}

#[test]
fn restoring_a_rule_is_recorded_separately_from_retiring_it() {
    // Otherwise a rule retired and restored ten times reads as ten retirements
    // and churn looks one-directional.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());
    let record = proposal(GardenProposalKind::RuleRetirement, "rule-1");

    record_proposal_applied(&context, &record, "rule-1");
    record_proposal_rolled_back(&context, &record);

    let events = recorded(dir.path());
    let rule_operations: Vec<&str> = events
        .iter()
        .filter_map(|event| event.identity("rule_operation"))
        .collect();
    assert!(rule_operations.contains(&"retire"));
    assert!(rule_operations.contains(&"restore"));

    let snapshot = derive_snapshot(None, &events);
    assert_eq!(
        snapshot.pooled("rule_retire_count").expect("derived").value,
        Some(1.0),
        "a restore must not count as a second retirement"
    );
}

#[test]
fn retiring_a_memory_does_not_record_rule_churn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());

    record_proposal_applied(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "mem-1"),
        "mem-1",
    );

    let events = recorded(dir.path());
    assert!(
        events
            .iter()
            .all(|event| event.identity("rule_operation").is_none()),
        "a memory retirement was counted as rule churn"
    );
}

#[test]
fn re_emitting_the_same_step_does_not_add_a_second_row() {
    // Event ids are derived from the proposal and the operation, so a retried
    // step is recognised as a replay rather than doubling the count.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");

    record_proposal_decided(&context, &record, true);
    record_proposal_decided(&context, &record, true);

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    assert_eq!(
        snapshot
            .pooled("governed_proposal_acceptance_rate")
            .expect("derived")
            .sample_count,
        1
    );
}

#[test]
fn events_are_segmented_by_cohort_as_well_as_pooled() {
    // The gate judges every segment, so an event that only ever lands in the
    // pooled cohort would be invisible to it.
    let dir = tempfile::tempdir().expect("tempdir");
    let context = context(dir.path());
    record_proposal_decided(
        &context,
        &proposal(GardenProposalKind::MemoryRetirement, "a"),
        true,
    );

    let snapshot = derive_snapshot(None, &recorded(dir.path()));
    let segmented: Vec<&archon_cognitive::metrics::derive::DerivedMetric> = snapshot
        .metrics
        .iter()
        .filter(|metric| {
            metric.metric_name == "governed_proposal_acceptance_rate"
                && metric.cohort != MetricCohort::pooled()
        })
        .collect();

    assert!(
        !segmented.is_empty(),
        "the observation landed only in the pooled cohort"
    );
}

#[test]
fn a_missing_cognitive_store_does_not_panic_or_block() {
    // Emission is best effort: a measurement lost is recoverable, a governed
    // decision lost is not.
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("nowhere").join("deeper");

    record_proposal_decided(
        &context(&missing),
        &proposal(GardenProposalKind::MemoryRetirement, "a"),
        true,
    );
}
