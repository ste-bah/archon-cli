use archon_cognitive::attribution::followup::{
    COHORT_ENTRY_BUCKET_DAYS, FOLLOWUP_WINDOW_TURNS, cohort_entry_window_id, match_stratum_id,
};
use chrono::{TimeZone, Utc};

use super::*;

fn attribution(cohort: &str, turn_number: u64) -> AttributedCorrection {
    AttributedCorrection {
        correction_id: "corr-1".into(),
        session_id: "s1".into(),
        turn_number,
        attribution_cohort: cohort.into(),
        attributed_cause_action_class: if cohort == "accepted" {
            "tool_run"
        } else {
            "none"
        }
        .into(),
        causal_candidate_id: "cc-1".into(),
        task_class: "conversation".into(),
        model_id: "test-model".into(),
        policy_version: "causal-attribution/v1".into(),
        recorded_at: Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap(),
    }
}

/// The follow-up window is a bound, not a suggestion.
#[test]
fn opportunities_outside_the_window_are_not_opportunities() {
    let subject = attribution("accepted", 4);

    assert!(subject.covers("s1", 5));
    assert!(subject.covers("s1", 4 + FOLLOWUP_WINDOW_TURNS));
    assert!(!subject.covers("s1", 4 + FOLLOWUP_WINDOW_TURNS + 1));
    // The correction's own turn is not a chance for the mistake to recur.
    assert!(!subject.covers("s1", 4));
    assert!(!subject.covers("s1", 3));
    // And neither is another session's turn.
    assert!(!subject.covers("another-session", 5));
}

/// Accepted and comparator opportunities must be able to share a stratum, or
/// the roadmap's eligibility filter reports zero eligible strata forever.
#[test]
fn accepted_and_comparator_opportunities_share_a_stratum() {
    let accepted = attribution("accepted", 4);
    let unattributed = attribution("unattributed", 4);

    let stratum = |subject: &AttributedCorrection| {
        match_stratum_id(
            &subject.task_class,
            "tool_run",
            &subject.model_id,
            &subject.policy_version,
            &subject.cohort_entry_window_id(),
        )
    };

    assert_eq!(stratum(&accepted), stratum(&unattributed));
    assert!(!accepted.is_comparator());
    assert!(unattributed.is_comparator());
    assert!(attribution("abstained", 4).is_comparator());
}

/// Cohort-entry buckets are FIXED calendar windows, not windows relative to
/// whichever correction was seen first.
///
/// The distinction matters: a relative bucket would make two corrections'
/// pooling depend on the order they were observed in, so the same corpus would
/// stratify differently on a re-run.
#[test]
fn the_cohort_entry_bucket_is_a_fixed_calendar_window() {
    let morning = Utc.with_ymd_and_hms(2026, 8, 10, 1, 0, 0).unwrap();
    let evening = Utc.with_ymd_and_hms(2026, 8, 10, 23, 0, 0).unwrap();
    assert_eq!(
        cohort_entry_window_id(morning),
        cohort_entry_window_id(evening),
        "one calendar day cannot straddle two buckets"
    );

    // Exactly one bucket width apart always lands in adjacent buckets, whatever
    // the starting date, which is what "fixed" means.
    for offset in 0..COHORT_ENTRY_BUCKET_DAYS {
        let start = morning + chrono::Duration::days(offset);
        assert_ne!(
            cohort_entry_window_id(start),
            cohort_entry_window_id(start + chrono::Duration::days(COHORT_ENTRY_BUCKET_DAYS)),
            "offset {offset} pooled two buckets"
        );
    }
}

// ── live wiring ──────────────────────────────────────────────

fn cognitive_store(root: &std::path::Path) -> archon_cognitive::PersistentCognitiveStore {
    archon_cognitive::PersistentCognitiveStore::open(root.join(".archon").join("cognitive"))
        .expect("cognitive store")
}

fn followup_rows(
    store: &archon_cognitive::PersistentCognitiveStore,
) -> Vec<archon_cognitive::CognitiveMetricEvent> {
    archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())
        .expect("metric event store")
        .events()
        .expect("read metric events")
        .into_iter()
        .filter(|event| {
            event.event_kind == archon_cognitive::MetricEventKind::AttributionFollowupEvaluated
        })
        .collect()
}

fn correction_transcript(is_error: bool) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"role": "user", "content": "please run the build"}),
        serde_json::json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "tu-1", "name": "RunShell", "input": {"cmd": "build"}}
        ]}),
        serde_json::json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "tu-1", "content": "error", "is_error": is_error}
        ]}),
        serde_json::json!({"role": "user", "content": "no, that broke the build"}),
    ]
}

/// The whole pass end to end: a correction is attributed on turn 2, turn 3 runs
/// a tool and fails, and that turn is recorded as a repeated verified failure
/// for the correction.
#[tokio::test]
async fn a_later_failing_turn_is_recorded_as_a_repeated_verified_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "followup-session".into();
    agent.turn_number = 2;
    agent.state.messages = correction_transcript(true);
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> =
        std::sync::Arc::new(archon_memory::graph::MemoryGraph::in_memory().expect("graph"));

    agent
        .detect_and_record_correction("no, that broke the build", &memory)
        .await;

    // Turn 3: the agent runs a tool again and it fails again.
    agent.turn_number = 3;
    agent
        .state
        .messages
        .push(serde_json::json!({"role": "user", "content": "try again"}));
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "tu-2", "name": "RunShell", "input": {"cmd": "build"}}]
    }));
    agent.state.messages.push(serde_json::json!({
        "role": "user",
        "content": [{"type": "tool_result", "tool_use_id": "tu-2", "content": "error", "is_error": true}]
    }));

    let written = agent
        .record_attribution_followup("try again")
        .await
        .expect("the follow-up pass completed");
    assert_eq!(written, 1);

    let store = cognitive_store(temp.path());
    let rows = followup_rows(&store);
    assert_eq!(rows.len(), 1);
    rows[0]
        .validate()
        .expect("the row the store accepted is valid");
    assert_eq!(rows[0].outcome_status, "failed");
    assert_eq!(rows[0].identity("attribution_cohort"), Some("accepted"));
    assert_eq!(rows[0].identity("followup_comparator"), Some("false"));
    assert_eq!(rows[0].identity("cause_action_class"), Some("tool_run"));
    assert_eq!(rows[0].turn_number, 3);
    for key in archon_cognitive::MetricEventKind::AttributionFollowupEvaluated.required_identities()
    {
        assert!(
            rows[0].identity(key).is_some_and(|value| !value.is_empty()),
            "mandatory identity `{key}` is missing"
        );
    }
}

/// A turn that ran nothing is not an opportunity.
#[tokio::test]
async fn a_turn_that_ran_no_tools_records_no_opportunity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "quiet-session".into();
    agent.turn_number = 2;
    agent.state.messages = correction_transcript(true);
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> =
        std::sync::Arc::new(archon_memory::graph::MemoryGraph::in_memory().expect("graph"));
    agent
        .detect_and_record_correction("no, that broke the build", &memory)
        .await;

    agent.turn_number = 3;
    agent
        .state
        .messages
        .push(serde_json::json!({"role": "user", "content": "thanks"}));

    assert_eq!(agent.record_attribution_followup("thanks").await, Some(0));
    assert!(followup_rows(&cognitive_store(temp.path())).is_empty());
}

/// The same opportunity evaluated twice is one row, not two.
#[tokio::test]
async fn a_replayed_followup_pass_writes_no_second_row() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "replay-followup".into();
    agent.turn_number = 2;
    agent.state.messages = correction_transcript(true);
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> =
        std::sync::Arc::new(archon_memory::graph::MemoryGraph::in_memory().expect("graph"));
    agent
        .detect_and_record_correction("no, that broke the build", &memory)
        .await;

    agent.turn_number = 3;
    agent
        .state
        .messages
        .push(serde_json::json!({"role": "user", "content": "try again"}));
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "tu-2", "name": "RunShell", "input": {}}]
    }));
    agent.state.messages.push(serde_json::json!({
        "role": "user",
        "content": [{"type": "tool_result", "tool_use_id": "tu-2", "content": "ok", "is_error": false}]
    }));

    assert_eq!(
        agent.record_attribution_followup("try again").await,
        Some(1)
    );
    assert_eq!(
        agent.record_attribution_followup("try again").await,
        Some(0)
    );

    let rows = followup_rows(&cognitive_store(temp.path()));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome_status, "passed");
}
