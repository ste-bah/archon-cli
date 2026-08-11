use std::sync::Arc;

use archon_memory::MemoryTrait;
use archon_memory::graph::MemoryGraph;
use serde_json::json;

use super::*;

fn graph() -> Arc<dyn MemoryTrait> {
    Arc::new(MemoryGraph::in_memory().expect("graph")) as Arc<dyn MemoryTrait>
}

fn cognitive_store(root: &std::path::Path) -> archon_cognitive::PersistentCognitiveStore {
    archon_cognitive::PersistentCognitiveStore::open(root.join(".archon").join("cognitive"))
        .expect("cognitive store")
}

/// Read the rows back through a second handle on the same store.
fn rows(
    store: &archon_cognitive::PersistentCognitiveStore,
) -> Vec<archon_cognitive::CognitiveMetricEvent> {
    archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())
        .expect("metric event store")
        .events()
        .expect("read metric events")
}

fn attribution_rows(
    store: &archon_cognitive::PersistentCognitiveStore,
) -> Vec<archon_cognitive::CognitiveMetricEvent> {
    rows(store)
        .into_iter()
        .filter(|event| event.event_kind == archon_cognitive::MetricEventKind::AttributionEvaluated)
        .collect()
}

fn identity<'a>(event: &'a archon_cognitive::CognitiveMetricEvent, key: &str) -> &'a str {
    event.identity(key).unwrap_or_default()
}

/// The one attribution row this turn produced.
///
/// No polling and no sleeping. `detect_and_record_correction` awaits both the
/// planning task and the commit task to completion, so by the time it returns
/// the row is either in the store or was deliberately withheld. A test that
/// slept here would be asserting a timing margin the machine never agreed to.
fn attribution_row(
    store: &archon_cognitive::PersistentCognitiveStore,
) -> archon_cognitive::CognitiveMetricEvent {
    let mut rows = attribution_rows(store);
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one attribution row, got {}",
        rows.len()
    );
    rows.remove(0)
}

/// Everything is `Unknown` because `test_agent` builds an empty registry, which
/// is the honest answer for a tool that is not registered.
fn unknown_effect(_name: &str, _input: &serde_json::Value) -> ActionEffectClass {
    ActionEffectClass::Unknown
}

// -- reconstructing the action window -------------------------

fn transcript(result_content: &str, is_error: bool) -> Vec<serde_json::Value> {
    vec![
        json!({"role": "user", "content": "please run the build"}),
        json!({"role": "assistant", "content": [
            {"type": "text", "text": "running it"},
            {"type": "tool_use", "id": "tu-1", "name": "RunShell", "input": {"cmd": "build"}},
            {"type": "tool_use", "id": "tu-2", "name": "ReadFile", "input": {"path": "log"}}
        ]}),
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "tu-1", "content": result_content, "is_error": is_error},
            {"type": "tool_result", "tool_use_id": "tu-2", "content": "ok", "is_error": false}
        ]}),
        json!({"role": "user", "content": "no, that broke the build"}),
    ]
}

#[test]
fn the_previous_turns_tool_runs_are_reconstructed_from_the_transcript() {
    let messages = transcript("error: build failed", true);

    let runs = observed_tool_runs(&messages, "s1", 2, &unknown_effect);

    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].tool_use_id, "tu-1");
    assert_eq!(runs[0].tool_name, "RunShell");
    assert_eq!(
        runs[0].turn_number, 1,
        "the corrected turn, not the current one"
    );
    assert_eq!(runs[0].ordinal, 0);
    assert!(runs[0].failed, "tool_result.is_error is the failure signal");
    assert!(!runs[0].blocked);
    assert_eq!(runs[0].action_attempt_id(), "s1:tu-1:1");
    assert_eq!(runs[1].tool_use_id, "tu-2");
    assert_eq!(runs[1].ordinal, 1);
    assert!(!runs[1].failed);
}

/// A refusal and a failure are different evidence, so they are recorded
/// differently. Both are errors; only one of them ran.
#[test]
fn a_refused_tool_is_recorded_as_blocked_rather_than_merely_failed() {
    let messages = transcript(
        "Permission denied for tool 'RunShell'. Current mode: ask.",
        true,
    );

    let runs = observed_tool_runs(&messages, "s1", 2, &unknown_effect);

    assert!(runs[0].failed);
    assert!(runs[0].blocked);
}

/// The current turn's own actions are not in the window: they had not happened
/// when the user typed the correction.
#[test]
fn a_first_turn_correction_has_no_prior_actions_to_reconstruct() {
    let messages = vec![json!({"role": "user", "content": "no, that is wrong"})];

    assert!(observed_tool_runs(&messages, "s1", 1, &unknown_effect).is_empty());
}

/// The effect class comes from the tool's own permission level, so a tool that
/// is no longer registered is `Unknown` -- never `Read`.
#[test]
fn an_unregistered_tool_has_an_unknown_effect_class_not_a_harmless_one() {
    let registry = crate::dispatch::ToolRegistry::new();

    assert_eq!(
        effect_class_of(&registry, "NotRegistered", &json!({})),
        ActionEffectClass::Unknown
    );
}

// -- the live call site ---------------------------------------

/// The whole point: a correction on the real turn path produces an
/// `attribution_evaluated` row that names the action it blames.
#[tokio::test]
async fn a_correction_after_a_failed_tool_writes_an_accepted_attribution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "attribution-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let memory = graph();

    agent
        .detect_and_record_correction("no, that broke the build", &memory)
        .await;

    let store = cognitive_store(temp.path());
    let event = attribution_row(&store);

    event
        .validate()
        .expect("the row the store accepted is valid");
    assert_eq!(identity(&event, "accepted"), "true");
    assert_eq!(identity(&event, "abstained"), "false");
    assert_eq!(identity(&event, "attribution_cohort"), "accepted");
    assert_eq!(identity(&event, "cause_action_class"), "tool_run");
    assert_eq!(identity(&event, "tool_use_id"), "tu-1");
    assert_eq!(
        identity(&event, "action_attempt_id"),
        "attribution-session:tu-1:1"
    );
    assert_eq!(identity(&event, "correction_type"), "factual_error");
    assert_eq!(event.session_id, "attribution-session");
    assert_eq!(event.turn_number, 2);
    assert!(
        event
            .evidence_refs
            .contains(&"tool_result:is_error".to_string())
    );

    // The correction the row points at is the one that was actually written.
    let stored = memory
        .search_memories(&archon_memory::types::SearchFilter {
            memory_type: Some(archon_memory::types::MemoryType::Correction),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(stored.len(), 1);
    assert_eq!(identity(&event, "correction_id"), stored[0].id);
}

/// Shadow containment, read back out of the row rather than asserted in prose.
#[tokio::test]
async fn the_live_row_records_that_attribution_mutated_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "shadow-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    agent.set_cognitive_store(cognitive_store(temp.path()));

    agent
        .detect_and_record_correction("no, that broke the build", &graph())
        .await;

    let store = cognitive_store(temp.path());
    let event = attribution_row(&store);

    assert_eq!(identity(&event, "attribution_mode"), "shadow");
    assert_eq!(identity(&event, "mutation_source"), "none");
    assert_eq!(
        identity(&event, "attribution_version"),
        archon_cognitive::CAUSAL_ATTRIBUTION_VERSION
    );
    // Unadjudicated, and therefore not counted as a correct link by anything
    // that follows the roadmap's precision rule.
    assert_ne!(
        identity(&event, "causal_candidate_id"),
        identity(&event, "adjudicated_causal_candidate_id")
    );
}

/// A correction with nothing before it is recorded as unattributed, not dropped
/// and not pinned to something. The comparator cohort the promotion gate needs
/// only exists if these rows are written.
#[tokio::test]
async fn a_correction_with_no_preceding_actions_writes_an_unattributed_row() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "empty-window-session".into();
    agent.turn_number = 1;
    agent.state.messages = vec![json!({"role": "user", "content": "no, that is wrong"})];
    agent.set_cognitive_store(cognitive_store(temp.path()));

    agent
        .detect_and_record_correction("no, that is wrong", &graph())
        .await;

    let store = cognitive_store(temp.path());
    let event = attribution_row(&store);

    assert_eq!(identity(&event, "attribution_cohort"), "unattributed");
    assert_eq!(identity(&event, "accepted"), "false");
    assert_eq!(identity(&event, "abstained"), "false");
    assert_eq!(identity(&event, "cause_action_class"), "none");
    assert_eq!(identity(&event, "candidate_population"), "0");
}

/// A turn the heuristic declined records an R3 shadow label and no attribution.
/// Attribution consumes high-confidence corrections; there is no correction here
/// to consume.
#[tokio::test]
async fn an_abstained_turn_produces_no_attribution_row() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "abstain-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    agent.set_cognitive_store(cognitive_store(temp.path()));

    agent
        .detect_and_record_correction("that's not what I meant", &graph())
        .await;

    // No polling: the attribution path is awaited end to end, so if no row
    // exists when the call returns, none ever will. (The R3 shadow label on the
    // same turn IS fire-and-forget, which is why this test says nothing about
    // it — `correction_intake_tests` owns that claim.)
    assert!(attribution_rows(&cognitive_store(temp.path())).is_empty());
}

/// One correction, one attribution row, however many times the write is retried.
#[tokio::test]
async fn a_replayed_attribution_write_is_not_counted_twice() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = cognitive_store(temp.path());
    let observation = AttributionObservation {
        session_id: "replay-session".into(),
        task_class: "conversation".into(),
        model_id: "test-model".into(),
        provenance: archon_consciousness::correction_provenance::CorrectionProvenance::from_record(
            &archon_consciousness::corrections::Correction {
                id: "corr-replay".into(),
                correction_type: archon_consciousness::corrections::CorrectionType::FactualError,
                content: "no, that broke the build".into(),
                context: archon_consciousness::correction_provenance::immediate_turn_context(2),
                severity: 1.5,
                rule_id: None,
                timestamp: chrono::Utc::now(),
            },
        ),
        correction_content: "no, that broke the build".into(),
        tool_runs: observed_tool_runs(
            &transcript("error: build failed", true),
            "replay-session",
            2,
            &unknown_effect,
        ),
        ledger_dir: None,
    };

    let plan = plan_correction_attribution(&store, &observation);
    let first = commit_correction_attribution(&store, &plan).expect("first write");
    let replayed = commit_correction_attribution(&store, &plan).expect("replayed write");

    assert!(first.accepted);
    assert_eq!(
        first, replayed,
        "a replay must reach the same verdict, not a second observation"
    );
    assert_eq!(attribution_rows(&store).len(), 1);
    // The lesson is corroborated by its own correction once, not twice.
    let lesson_id = first
        .lesson_id
        .expect("an accepted attribution derives a lesson");
    let lesson = archon_cognitive::attribution::lesson::read_causal_lesson(store.db(), &lesson_id)
        .expect("read lesson")
        .expect("lesson exists");
    assert_eq!(lesson.corroboration_count, 1);
}

// The item-5 reinforcement-gate and lesson-edge tests live in a child module to
// keep this file under the size gate.
#[path = "correction_attribution_tests/reinforcement_gate.rs"]
mod reinforcement_gate;

/// The two derived metrics that have existed since the R8 schema landed with
/// nothing able to produce them.
#[tokio::test]
async fn the_written_rows_make_the_causal_attribution_metrics_derivable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = "derive-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    agent.set_cognitive_store(cognitive_store(temp.path()));

    agent
        .detect_and_record_correction("no, that broke the build", &graph())
        .await;

    let store = cognitive_store(temp.path());
    let event = attribution_row(&store);
    let window = archon_cognitive::attribution::event::attribution_window(event.created_at);
    let snapshot = archon_cognitive::metrics::derive_snapshot(Some(&window), &rows(&store));

    let accept_rate = snapshot
        .pooled("causal_attribution_accept_rate")
        .expect("accept rate is derivable from the rows just written");
    assert_eq!(accept_rate.value, Some(1.0));
    assert_eq!(accept_rate.denominator, 1.0);
    let abstention_rate = snapshot
        .pooled("causal_attribution_abstention_rate")
        .expect("abstention rate is derivable");
    assert_eq!(abstention_rate.value, Some(0.0));
}
