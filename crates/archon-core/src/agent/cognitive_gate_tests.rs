use std::sync::Arc;

use archon_cognitive::{CognitiveConfig, DecisionStore};
use archon_llm::provider::{LlmError, LlmProvider, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;

use super::*;

struct QuietLlmProvider;

#[async_trait::async_trait]
impl LlmProvider for QuietLlmProvider {
    fn name(&self) -> &str {
        "quiet"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

fn agent_with_cognitive_store(root: &std::path::Path) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    config.session_id = "cognitive-persist-test".to_owned();
    config.context.prompt_cache = false;
    let mut agent = Agent::new(
        Arc::new(QuietLlmProvider),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
    );
    let store =
        archon_cognitive::PersistentCognitiveStore::open(root.join(".archon").join("cognitive"))
            .expect("cognitive store");
    agent.set_cognitive_store(store);
    agent
}

fn enable_executive_runtime(agent: &mut Agent, root: &std::path::Path) {
    let config = CognitiveConfig {
        enabled: true,
        record_decisions: true,
        record_reflections: true,
        // The advisory and the shadow observer are both abandoned when they
        // exceed this, and the default 500ms is wall-clock on a box running
        // nine hundred other tests. These tests assert what the loop does, not
        // how fast the machine is; `a_slow_observation_is_abandoned_at_its_budget`
        // is where the budget itself is tested.
        max_pipeline_ms: 5_000,
        ..CognitiveConfig::default()
    };
    let policy = archon_cognitive::CognitivePolicy {
        enabled: true,
        max_autonomous_risk: "Medium".into(),
        ..archon_cognitive::CognitivePolicy::default()
    };
    agent.set_cognitive_executive(config, policy, root.join(".archon").join("cognitive"));
}

#[tokio::test]
async fn greeting_turn_records_compact_cognitive_situation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with_cognitive_store(temp.path());

    agent
        .process_message("hello")
        .await
        .expect("process message");

    let store = agent.cognitive_store.as_ref().expect("store");
    let store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(store.situation_count().expect("situation count"), 1);
    assert_eq!(store.decision_count().expect("decision count"), 0);
}

#[tokio::test]
async fn nontrivial_live_turn_records_executive_decision_without_claiming_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with_cognitive_store(temp.path());
    enable_executive_runtime(&mut agent, temp.path());

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    // Read once, with no polling: `ExecutiveLoop::run_advisory` records the
    // decision inside the bounded task the turn awaits, so by the time
    // `process_message` returns the row is already there. The previous free
    // function handed the decision to a detached task, which is why this used
    // to need a retry loop.
    let store =
        archon_cognitive::PersistentCognitiveStore::open(temp.path().join(".archon/cognitive"))
            .expect("cognitive store");
    let decisions = DecisionStore::new(
        store.db(),
        temp.path()
            .join(".archon/cognitive/cognitive-decisions.jsonl"),
    )
    .expect("decision store")
    .list_for_session("cognitive-persist-test", 10)
    .expect("executive decisions");

    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].verification_contract.is_some());
    assert!(decisions[0].user_visible_summary.contains("code_change"));
    let mut system = Vec::new();
    agent.inject_turn_requirements(&mut system);
    let executive_prompt = system
        .iter()
        .filter_map(|block| block["text"].as_str())
        .find(|text| text.contains("<cognitive-executive>"))
        .expect("executive advisory prompt");
    assert!(executive_prompt.contains("code_change"));
    assert!(executive_prompt.contains("planning guidance only"));
    assert!(
        !temp
            .path()
            .join(".archon/cognitive/cognitive-reflections.jsonl")
            .exists()
    );
}

fn shadow_rows(root: &std::path::Path) -> Vec<archon_cognitive::ShadowSummary> {
    let dir = root.join(".archon").join("cognitive");
    let store = archon_cognitive::PersistentCognitiveStore::open(&dir).expect("cognitive store");
    archon_cognitive::CognitiveInspection::new(store.db(), &dir)
        .expect("inspection")
        .shadow_decisions(10)
        .expect("shadow decisions")
}

/// Issue #76's acceptance in one test: the executive loop runs on a real turn,
/// its plan is persisted, and the real outcome is joined to it — without the
/// live decision or reflection surfaces gaining a row for work nobody did.
#[tokio::test]
async fn live_turn_runs_the_executive_loop_as_a_joined_shadow_observer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with_cognitive_store(temp.path());
    enable_executive_runtime(&mut agent, temp.path());

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    let rows = shadow_rows(temp.path());
    assert_eq!(rows.len(), 1, "the shadow loop did not run on a live turn");
    assert!(!rows[0].shadow_action.is_empty());
    assert!(rows[0].joined, "shadow was never joined");
    // The label is the live turn's: a turn with no tool calls answered
    // directly, whatever the no-op shadow executor reported.
    assert_eq!(rows[0].live_action, "answer_directly");
    assert!(rows[0].agreed.is_some(), "agreement not measured");
    assert!(rows[0].surprise.is_some(), "surprise not measured");

    // A mismatch alone is below the reflection threshold, so a clean turn
    // still writes no lesson.
    assert!(
        !temp
            .path()
            .join(".archon/cognitive/cognitive-reflections.jsonl")
            .exists()
    );
    assert!(
        temp.path()
            .join(".archon/cognitive/cognitive-shadow-decisions.jsonl")
            .exists()
    );
}

/// Without the executive runtime wired there is nothing to shadow, and the
/// turn must not acquire cognitive side effects it did not have before.
#[tokio::test]
async fn a_turn_without_the_executive_runtime_records_no_shadow() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with_cognitive_store(temp.path());

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    assert!(shadow_rows(temp.path()).is_empty());
}

/// The non-negotiable property: a slow observer cannot hold up the user.
#[tokio::test]
async fn a_slow_observation_is_abandoned_at_its_budget() {
    let budget_ms = 40;
    let started = std::time::Instant::now();

    let result = super::cognitive_gate::bounded_cognitive_observation(
        budget_ms,
        "test-slow-observation",
        move || {
            std::thread::sleep(std::time::Duration::from_millis(2_000));
            "observer finished"
        },
    )
    .await;

    assert!(result.is_none(), "a slow observer was allowed to block");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1_000),
        "waited {:?} for a {budget_ms}ms budget",
        started.elapsed()
    );
}

/// A panicking observer is a bug in the observer, not a failed turn.
#[tokio::test]
async fn a_panicking_observation_does_not_propagate() {
    let result = super::cognitive_gate::bounded_cognitive_observation(
        1_000,
        "test-panicking-observation",
        || panic!("observer panicked"),
    )
    .await;

    assert!(result.is_none());
}

// ── reflection-trigger correction confidence ─────────────────

use archon_consciousness::correction_classifier::{
    CorrectionClassification, EXPLICIT_PHRASE_CONFIDENCE, RATIONALE_ABSTAIN_BELOW_THRESHOLD,
    RATIONALE_ABSTAIN_NO_SIGNAL, RATIONALE_PROVIDER_JUDGED,
};

fn classification(
    is_correction: bool,
    confidence: f32,
    rationale_code: &str,
) -> CorrectionClassification {
    CorrectionClassification {
        is_correction,
        correction_type: None,
        confidence,
        rationale_code: rationale_code.to_owned(),
    }
}

/// The real classifier confidence reaches the trigger, replacing the constant
/// `0.9` every corrected turn used to report.
#[test]
fn a_recorded_correction_carries_the_classifier_confidence() {
    let explicit = classification(
        true,
        EXPLICIT_PHRASE_CONFIDENCE,
        "explicit_phrase.factual_error",
    );

    let confidence = super::cognitive_gate::correction_trigger_confidence(true, Some(&explicit))
        .expect("a recorded, positively classified correction has a confidence");

    assert_eq!(confidence, EXPLICIT_PHRASE_CONFIDENCE);
    assert_eq!(
        archon_cognitive::reflection_trigger::evaluate(&archon_cognitive::TurnSignals {
            correction_confidence: Some(confidence),
            ..archon_cognitive::TurnSignals::new(archon_cognitive::SituationKind::CodeChange)
        })
        .map(|triggered| triggered.trigger),
        Some(archon_cognitive::ReflectionTrigger::HighConfidenceCorrection),
    );
}

/// An abstention declines to answer. Reporting its confidence would let the
/// trigger read "I don't know" as a correction it happens to be unsure of.
#[test]
fn an_abstention_is_not_a_low_confidence_correction() {
    let no_signal = classification(false, 0.0, RATIONALE_ABSTAIN_NO_SIGNAL);
    let below = classification(false, 0.55, RATIONALE_ABSTAIN_BELOW_THRESHOLD);

    assert_eq!(
        super::cognitive_gate::correction_trigger_confidence(true, Some(&no_signal)),
        None
    );
    assert_eq!(
        super::cognitive_gate::correction_trigger_confidence(true, Some(&below)),
        None
    );
}

/// The failure the constant invited, inverted: a classifier certain the user
/// corrected NOTHING carries a high confidence too, and passing it through
/// would fire the strongest reflection trigger on a turn with no correction.
#[test]
fn a_confident_non_correction_never_arms_the_trigger() {
    let confident_no = classification(false, 0.95, RATIONALE_PROVIDER_JUDGED);

    assert_eq!(
        super::cognitive_gate::correction_trigger_confidence(true, Some(&confident_no)),
        None
    );
}

/// The classifier is shadow-only until its promotion gate passes, so it grades
/// a correction the live path recorded rather than declaring one itself.
#[test]
fn a_classification_the_live_path_did_not_record_stays_out() {
    let explicit = classification(
        true,
        EXPLICIT_PHRASE_CONFIDENCE,
        "explicit_phrase.factual_error",
    );

    assert_eq!(
        super::cognitive_gate::correction_trigger_confidence(false, Some(&explicit)),
        None
    );
    assert_eq!(
        super::cognitive_gate::correction_trigger_confidence(true, None),
        None
    );
}

#[test]
fn turn_tool_activity_counts_only_this_turn() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "earlier turn"}),
        serde_json::json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "t0", "name": "WebSearch", "input": {}}
        ]}),
        serde_json::json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t0", "content": "boom", "is_error": true}
        ]}),
        serde_json::json!({"role": "user", "content": "this turn"}),
        serde_json::json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "t1", "name": "Read", "input": {}},
            {"type": "tool_use", "id": "t2", "name": "Bash", "input": {}}
        ]}),
        serde_json::json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false},
            {"type": "tool_result", "tool_use_id": "t2", "content": "boom", "is_error": true}
        ]}),
    ];

    let (tools, failures) = super::cognitive_gate::turn_tool_activity(&messages, "this turn");

    assert_eq!(tools, vec!["Read".to_string(), "Bash".to_string()]);
    assert_eq!(failures, 1, "a previous turn's failure leaked in");
}
