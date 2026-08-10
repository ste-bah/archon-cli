use chrono::TimeZone;

use super::*;
use crate::attribution::event::attribution_event;
use crate::attribution::input::{
    ActionEffectClass, AttributionInput, CorrectionUnderReview, ObservedToolRun,
};
use crate::attribution::{AttributionEngine, CAUSE_ACTION_CLASS_NONE};

const SESSION: &str = "adjudication-session";

fn recorded_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap()
}

fn store_root() -> (tempfile::TempDir, crate::store::PersistentCognitiveStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = crate::store::PersistentCognitiveStore::open(temp.path().join("cognitive"))
        .expect("cognitive store");
    (temp, store)
}

/// Write one shadow attribution row so there is something to adjudicate.
fn seed_attribution(
    store: &crate::store::PersistentCognitiveStore,
    correction_id: &str,
    failed: bool,
) {
    let mut run = ObservedToolRun {
        session_id: SESSION.into(),
        turn_number: 4,
        ordinal: 0,
        tool_use_id: "tu-1".into(),
        attempt: 1,
        tool_name: "RunShell".into(),
        input_summary: String::new(),
        effect_class: ActionEffectClass::Unknown,
        failed,
        blocked: false,
    };
    run.failed = failed;
    let input = AttributionInput {
        correction: CorrectionUnderReview {
            correction_id: correction_id.into(),
            session_id: SESSION.into(),
            turn_number: 5,
            correction_type_code: "factual_error".into(),
            summary: "no, that broke the build".into(),
            recorded_at: recorded_at(),
        },
        tool_runs: if failed { vec![run] } else { Vec::new() },
        decisions: Vec::new(),
    };
    let assessment = AttributionEngine.attribute(&input);
    let window = attribution_window(recorded_at());
    let event_store = MetricEventStore::new(store.db(), store.root()).expect("event store");
    event_store.declare_window(&window).expect("declare window");
    let cohort = MetricCohort::new("conversation", "test-model", CAUSAL_ATTRIBUTION_VERSION);
    event_store
        .record(&attribution_event(
            &input,
            &assessment,
            cohort,
            &window,
            None,
        ))
        .expect("seed attribution row");
}

fn verdict(candidate: Option<&str>) -> AttributionVerdict {
    AttributionVerdict {
        adjudicated_candidate_id: candidate.map(str::to_string),
        adjudicator: "reviewer".into(),
        note: String::new(),
    }
}

#[test]
fn a_shadow_row_is_listed_as_pending_with_the_candidates_to_choose_from() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-1", true);

    let pending = list_pending(store.db(), store.root(), 10).expect("list pending");

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].correction_id, "corr-1");
    assert_eq!(pending[0].attribution_cohort, "accepted");
    assert_eq!(pending[0].cause_action_class, "tool_run");
    assert!(
        pending[0]
            .ranked_candidate_ids
            .contains(&pending[0].proposed_candidate_id),
        "the proposal must be among the candidates offered: {:?}",
        pending[0].ranked_candidate_ids
    );
}

/// Agreeing with the engine is what makes precision computable at all.
#[test]
fn agreeing_with_the_proposal_records_a_correct_link() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-1", true);
    let pending = list_pending(store.db(), store.root(), 10).expect("list")[0].clone();

    record_adjudication(
        store.db(),
        store.root(),
        &pending,
        &verdict(Some(&pending.proposed_candidate_id)),
        recorded_at(),
    )
    .expect("record adjudication");

    let events = MetricEventStore::new(store.db(), store.root())
        .expect("store")
        .events()
        .expect("events");
    let row = events
        .iter()
        .find(|event| event.identity(ADJUDICATION_SCOPE).is_some())
        .expect("an adjudication row");
    assert_eq!(row.label_source, ADJUDICATION_LABEL_SOURCE);
    assert_eq!(row.identity(ADJUDICATION_SCOPE), Some("accepted"));
    assert_eq!(
        row.identity("causal_candidate_id"),
        row.identity("adjudicated_causal_candidate_id"),
        "an agreed link is exactly the equality the roadmap defines as correct"
    );
    row.validate().expect("the row the store accepted is valid");

    // Adjudicated once, so it drops out of the pending list.
    assert!(
        list_pending(store.db(), store.root(), 10)
            .expect("list")
            .is_empty()
    );
}

/// Disagreeing is the outcome precision exists to count.
#[test]
fn rejecting_the_proposal_records_an_incorrect_link() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-1", true);
    let pending = list_pending(store.db(), store.root(), 10).expect("list")[0].clone();

    record_adjudication(
        store.db(),
        store.root(),
        &pending,
        &verdict(None),
        recorded_at(),
    )
    .expect("record adjudication");

    let snapshot = crate::metrics::derive_snapshot(
        None,
        &MetricEventStore::new(store.db(), store.root())
            .expect("store")
            .events()
            .expect("events"),
    );
    let precision = snapshot
        .pooled("causal_attribution_precision")
        .expect("precision is derivable once something has been adjudicated");
    assert_eq!(precision.value, Some(0.0));
    assert_eq!(precision.denominator, 1.0);
}

/// Precision is defined over adjudicated rows only. An engine proposal cannot
/// enter its own denominator.
#[test]
fn precision_is_undefined_while_nothing_has_been_adjudicated() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-1", true);

    let snapshot = crate::metrics::derive_snapshot(
        None,
        &MetricEventStore::new(store.db(), store.root())
            .expect("store")
            .events()
            .expect("events"),
    );

    assert!(
        snapshot.pooled("causal_attribution_precision").is_none(),
        "an unadjudicated corpus must produce no precision at all"
    );
    // ...while the rates that do not need a verdict are already computable.
    assert!(snapshot.pooled("causal_attribution_accept_rate").is_some());
}

/// An abstention can be adjudicated too, and it is scoped out of accepted-link
/// precision rather than counted as a miss.
#[test]
fn an_adjudicated_refusal_is_scoped_out_of_accepted_link_precision() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-2", false);
    let pending = list_pending(store.db(), store.root(), 10).expect("list")[0].clone();
    assert_eq!(pending.attribution_cohort, "unattributed");
    assert_eq!(pending.cause_action_class, CAUSE_ACTION_CLASS_NONE);

    record_adjudication(
        store.db(),
        store.root(),
        &pending,
        &verdict(None),
        recorded_at(),
    )
    .expect("record adjudication");

    let snapshot = crate::metrics::derive_snapshot(
        None,
        &MetricEventStore::new(store.db(), store.root())
            .expect("store")
            .events()
            .expect("events"),
    );
    assert!(
        snapshot.pooled("causal_attribution_precision").is_none(),
        "only accepted links carry accepted-link precision"
    );
}

/// A replayed verdict is a replay; a changed verdict is refused rather than
/// silently overwriting the record of the first one.
#[test]
fn one_correction_gets_one_adjudication() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-1", true);
    let pending = list_pending(store.db(), store.root(), 10).expect("list")[0].clone();

    assert_eq!(
        record_adjudication(
            store.db(),
            store.root(),
            &pending,
            &verdict(None),
            recorded_at()
        )
        .expect("first verdict"),
        MetricWriteOutcome::Written
    );
    assert_eq!(
        record_adjudication(
            store.db(),
            store.root(),
            &pending,
            &verdict(None),
            recorded_at()
        )
        .expect("replayed verdict"),
        MetricWriteOutcome::DuplicateIgnored
    );
    assert!(
        record_adjudication(
            store.db(),
            store.root(),
            &pending,
            &verdict(Some(&pending.proposed_candidate_id)),
            recorded_at()
        )
        .is_err(),
        "a second, different verdict must not overwrite the first"
    );
}

#[test]
fn an_unnamed_adjudicator_is_refused() {
    let (_temp, store) = store_root();
    seed_attribution(&store, "corr-1", true);
    let pending = list_pending(store.db(), store.root(), 10).expect("list")[0].clone();

    assert!(
        record_adjudication(
            store.db(),
            store.root(),
            &pending,
            &AttributionVerdict {
                adjudicated_candidate_id: None,
                adjudicator: "   ".into(),
                note: String::new(),
            },
            recorded_at()
        )
        .is_err()
    );
}
