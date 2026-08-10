use super::*;
use crate::attribution::event::{
    ADJUDICATION_PENDING_PREFIX, ATTRIBUTION_LABEL_SOURCE, ATTRIBUTION_METRIC, NO_CAUSE_PREFIX,
    UNOBSERVED_PREFIX, attribution_event, attribution_event_id, attribution_window,
};
use crate::metrics::event::MetricEventKind;
use crate::metrics::window::MetricCohort;

fn cohort() -> MetricCohort {
    MetricCohort::new("conversation", "test-model", CAUSAL_ATTRIBUTION_VERSION)
}

fn event_for(subject: &AttributionInput) -> crate::metrics::event::CognitiveMetricEvent {
    let assessment = attribute(subject);
    let window = attribution_window(subject.correction.recorded_at);
    let lesson = crate::attribution::lesson::causal_lesson(
        subject,
        &assessment,
        "conversation",
        "test-model",
    );
    attribution_event(
        subject,
        &assessment,
        cohort(),
        &window,
        lesson.as_ref().map(|lesson| lesson.lesson_id.as_str()),
    )
}

fn identity<'a>(event: &'a crate::metrics::event::CognitiveMetricEvent, key: &str) -> &'a str {
    event.identity(key).unwrap_or_default()
}

fn accepted_input() -> AttributionInput {
    let mut failing = tool_run("tu-shell", "RunShell", 4, 0);
    failing.failed = true;
    input(
        correction("factual_error", "no, that broke the build", 5),
        vec![failing],
        vec![decision("dec-1", 4)],
    )
}

/// Every mandatory identity is present, and the store agrees.
///
/// `validate` is what the metric layer runs at write time, so calling it here
/// is the same check the store would make -- a missing identity fails the test
/// rather than being discovered as a silent write failure in production.
#[test]
fn an_accepted_attribution_carries_every_mandatory_identity() {
    let event = event_for(&accepted_input());

    event.validate().expect("event must be writable");
    assert_eq!(event.event_kind, MetricEventKind::AttributionEvaluated);
    for key in MetricEventKind::AttributionEvaluated.required_identities() {
        assert!(
            !identity(&event, key).is_empty(),
            "mandatory identity `{key}` is empty"
        );
    }
    assert_eq!(identity(&event, "correction_id"), "corr-1");
    assert_eq!(identity(&event, "decision_id"), "dec-1");
    assert_eq!(
        identity(&event, "action_attempt_id"),
        "attribution-session:tu-shell:1"
    );
    assert_eq!(identity(&event, "tool_use_id"), "tu-shell");
    assert_eq!(identity(&event, "cause_action_class"), "tool_run");
    assert_eq!(identity(&event, "accepted"), "true");
    assert_eq!(identity(&event, "abstained"), "false");
    assert_eq!(identity(&event, "attribution_cohort"), "accepted");
    assert_eq!(identity(&event, "candidate_rank"), "0");
    assert_eq!(event.metric_name, ATTRIBUTION_METRIC);
    assert_eq!(event.label_source, ATTRIBUTION_LABEL_SOURCE);
}

/// The honesty property.
///
/// The roadmap scores an accepted link as correct only when the proposed and
/// adjudicated candidate ids are equal. Nothing has adjudicated these, so the
/// adjudicated id is a sentinel that cannot equal the proposal. Defaulting it to
/// the proposal would make precision read 1.0 over a corpus nobody has looked
/// at, which is the failure this test exists to prevent.
#[test]
fn the_adjudicated_candidate_is_pending_and_never_equals_the_proposal() {
    let event = event_for(&accepted_input());

    let proposed = identity(&event, "causal_candidate_id");
    let adjudicated = identity(&event, "adjudicated_causal_candidate_id");
    assert!(adjudicated.starts_with(ADJUDICATION_PENDING_PREFIX));
    assert_ne!(proposed, adjudicated);
    assert!(
        identity(&event, "attribution_adjudication_id").starts_with(ADJUDICATION_PENDING_PREFIX)
    );
}

/// The row says shadow, so "nothing was mutated" is readable from the corpus
/// rather than asserted in a comment.
#[test]
fn every_row_records_that_nothing_was_mutated() {
    for subject in [accepted_input(), abstained_input(), unattributed_input()] {
        let event = event_for(&subject);
        assert_eq!(identity(&event, "attribution_mode"), "shadow");
        assert_eq!(identity(&event, "mutation_source"), "none");
        assert_eq!(
            identity(&event, "attribution_version"),
            CAUSAL_ATTRIBUTION_VERSION
        );
    }
}

fn abstained_input() -> AttributionInput {
    let mut first = tool_run("tu-a", "WriteFile", 4, 0);
    first.effect_class = ActionEffectClass::Mutate;
    first.input_summary = "config.toml".into();
    let mut second = tool_run("tu-b", "WriteFile", 4, 1);
    second.effect_class = ActionEffectClass::Mutate;
    second.input_summary = "config.toml".into();
    input(
        correction("factual_error", "no, the config file is wrong", 5),
        vec![first, second],
        Vec::new(),
    )
}

fn unattributed_input() -> AttributionInput {
    input(
        correction("factual_error", "no, that is wrong", 5),
        Vec::new(),
        Vec::new(),
    )
}

/// An abstention is written, is writable, and names no cause.
#[test]
fn an_abstention_is_a_first_class_row_that_names_no_cause() {
    let event = event_for(&abstained_input());

    event.validate().expect("event must be writable");
    assert_eq!(identity(&event, "accepted"), "false");
    assert_eq!(identity(&event, "abstained"), "true");
    assert_eq!(identity(&event, "attribution_cohort"), "abstained");
    assert_eq!(identity(&event, "cause_action_class"), "none");
    assert!(identity(&event, "causal_candidate_id").starts_with(NO_CAUSE_PREFIX));
    assert!(identity(&event, "action_attempt_id").starts_with(UNOBSERVED_PREFIX));
    assert_eq!(identity(&event, "candidate_rank"), "none");
    // The candidates that lost are still on the row: that is what makes the
    // refusal reviewable rather than an opaque "no".
    assert_eq!(identity(&event, "candidate_population"), "2");
    assert!(identity(&event, "ranked_candidate_ids").contains("tu-"));
}

/// An unattributed correction is recorded, not dropped. The follow-up
/// comparison the promotion gate rests on needs this cohort to exist.
#[test]
fn an_unattributed_correction_is_still_recorded() {
    let event = event_for(&unattributed_input());

    event.validate().expect("event must be writable");
    assert_eq!(identity(&event, "attribution_cohort"), "unattributed");
    assert_eq!(identity(&event, "accepted"), "false");
    // Unattributed is NOT an abstention: the engine never got as far as
    // declining between candidates, and pooling the two would corrupt the
    // abstention rate.
    assert_eq!(identity(&event, "abstained"), "false");
    assert_eq!(identity(&event, "candidate_population"), "0");
    assert_eq!(identity(&event, "ranked_candidate_ids"), "none");
    assert!(identity(&event, "decision_id").starts_with(UNOBSERVED_PREFIX));
}

/// One correction, one row, whatever happens upstream.
#[test]
fn the_row_identity_is_derived_from_the_correction_and_the_procedure() {
    let subject = accepted_input();
    let first = event_for(&subject);
    let second = event_for(&subject);

    assert_eq!(first.metric_event_id, attribution_event_id("corr-1"));
    assert!(first.metric_event_id.contains(CAUSAL_ATTRIBUTION_VERSION));
    assert_eq!(first, second, "a replay must be byte-identical");
    assert_eq!(
        first.created_at, subject.correction.recorded_at,
        "the row is timestamped from the correction, so a replay after a \
         restart is still a replay"
    );
}

/// The window is a pure function of the correction's own timestamp.
#[test]
fn the_evaluation_window_is_the_utc_day_of_the_correction() {
    let subject = accepted_input();
    let window = attribution_window(subject.correction.recorded_at);

    assert_eq!(window.evaluation_window_id, "causal-attribution-2026-08-10");
    assert!(window.contains(subject.correction.recorded_at));
    window.validate().expect("window must be declarable");
    assert_eq!(
        window,
        attribution_window(subject.correction.recorded_at),
        "a redeclaration must produce identical bounds"
    );
}

/// Evidence refs are join keys, not prose.
#[test]
fn evidence_refs_name_the_things_an_adjudicator_would_open() {
    let event = event_for(&accepted_input());

    assert!(
        event
            .evidence_refs
            .contains(&"correction:corr-1".to_string())
    );
    assert!(
        event
            .evidence_refs
            .contains(&"tool_use:tu-shell".to_string())
    );
    assert!(
        event
            .evidence_refs
            .contains(&"tool_result:is_error".to_string())
    );
    assert!(
        event
            .evidence_refs
            .contains(&"evidence:deterministic_failure".to_string())
    );
}
