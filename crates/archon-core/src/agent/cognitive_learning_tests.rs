//! Live-turn tests for issues #80 and #81 (child module via `#[path]`).
//!
//! These run `Agent::process_message` end to end against a real cognitive store,
//! so what they prove is that the emitters have a call site the turn path
//! actually reaches — not merely that the functions behave when called.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_cognitive::{CognitiveConfig, PersistentCognitiveStore};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::ContentBlockType;
use cozo::ScriptMutability;

use super::*;

const SESSION: &str = "cognitive-learning-test";

/// A provider that says nothing. The turn completes having executed no tools,
/// which is the "nothing deterministic to verify" case.
struct QuietProvider;

/// A provider that asks for one tool on its first round and nothing afterwards.
///
/// The tool is not in the (empty) registry, so the agent records an error tool
/// result — a deterministic execution failure, which is exactly the evidence the
/// calibration metric is defined over.
struct OneFailingToolProvider {
    rounds: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmProvider for QuietProvider {
    fn name(&self) -> &str {
        "quiet"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("QuietProvider is streaming-only")
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }
}

#[async_trait::async_trait]
impl LlmProvider for OneFailingToolProvider {
    fn name(&self) -> &str {
        "one-failing-tool"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("OneFailingToolProvider is streaming-only")
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        if self.rounds.fetch_add(1, Ordering::SeqCst) == 0 {
            let _ = tx
                .send(StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some("tool-1".to_string()),
                    tool_name: Some("Read".to_string()),
                })
                .await;
            let _ = tx
                .send(StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: "{\"file_path\":\"nowhere.txt\"}".to_string(),
                })
                .await;
        }
        Ok(rx)
    }
}

fn agent_with(provider: Arc<dyn LlmProvider>, root: &std::path::Path) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut config = AgentConfig {
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    config.session_id = SESSION.to_owned();
    config.context.prompt_cache = false;
    let mut agent = Agent::new(
        provider,
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
    );
    let dir = root.join(".archon").join("cognitive");
    agent.set_cognitive_store(PersistentCognitiveStore::open(&dir).expect("cognitive store"));
    agent.set_cognitive_executive(
        CognitiveConfig {
            enabled: true,
            record_decisions: true,
            record_reflections: true,
            max_pipeline_ms: 5_000,
            ..CognitiveConfig::default()
        },
        archon_cognitive::CognitivePolicy {
            enabled: true,
            max_autonomous_risk: "Medium".into(),
            ..archon_cognitive::CognitivePolicy::default()
        },
        dir,
    );
    agent
}

fn run_script(agent: &Agent, script: &str) {
    let store = agent.cognitive_store.as_ref().expect("store");
    let store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    store
        .db()
        .run_script(script, Default::default(), ScriptMutability::Mutable)
        .expect("seed script");
}

fn seed_trust_fact(agent: &Agent, domain: &str, confidence: f64, evidence: i64) {
    run_script(
        agent,
        &format!(
            "?[fact_id, domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at] <- \
             [['domain_trust:{domain}', '{domain}', 'domain_trust', 'measured', {confidence}, {evidence}, '2026-01-01T00:00:00Z', '', '2026-01-01T00:00:00Z']]
             :put self_model_facts {{ fact_id => domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at }}"
        ),
    );
}

fn seed_triggered_reflection(agent: &Agent, id: &str) {
    run_script(
        agent,
        &format!(
            "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
             [['{id}', '{SESSION}', 1, 'd-{id}', 'code_change', '', '', '', 'failure', 'code_change: repeated tool failure should stop retrying', false, '', '2026-01-01T00:00:00Z']]
             :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}"
        ),
    );
    run_script(
        agent,
        &format!(
            "?[reflection_id, trigger, confidence, evidence_refs_json, created_at] <- \
             [['{id}', 'repeated_tool_failure', 0.75, '[]', '2026-01-01T00:00:00Z']]
             :put cognitive_reflection_evidence {{ reflection_id => trigger, confidence, evidence_refs_json, created_at }}"
        ),
    );
}

fn read_rows(root: &std::path::Path, script: &str) -> Vec<Vec<cozo::DataValue>> {
    let store =
        PersistentCognitiveStore::open(root.join(".archon").join("cognitive")).expect("reopen");
    store
        .db()
        .run_script(script, Default::default(), ScriptMutability::Immutable)
        .expect("read script")
        .rows
}

fn injected_block(agent: &Agent) -> Option<String> {
    let mut system = Vec::new();
    agent.inject_turn_requirements(&mut system);
    system
        .iter()
        .filter_map(|block| block["text"].as_str())
        .find(|text| text.contains("<self_model_briefing>") || text.contains("Unresolved lessons"))
        .map(str::to_owned)
}

// ── #80(a): the prediction exists before the action, and is graded after ──

/// The live call site. A turn on a domain the self-model has measured writes its
/// prediction before the request and resolves it after finalisation, in one
/// `process_message`.
#[tokio::test]
async fn a_live_turn_records_and_resolves_a_self_model_prediction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with(Arc::new(QuietProvider), temp.path());
    seed_trust_fact(&agent, "coding", 0.7, 11);

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    let rows = read_rows(
        temp.path(),
        "?[prediction_id, predicted_success_probability, resolved, verified_outcome, self_model_fact_id] := \
         *self_model_predictions{prediction_id, predicted_success_probability, resolved, verified_outcome, self_model_fact_id}",
    );
    assert_eq!(rows.len(), 1, "the pre-action prediction never ran");
    assert!((rows[0][1].get_float().unwrap() - 0.7).abs() < 1e-6);
    assert_eq!(
        rows[0][2].get_bool(),
        Some(true),
        "prediction never resolved"
    );
    // This turn executed no tools, so there is nothing deterministic to verify
    // and the prediction stays out of the calibration population.
    assert_eq!(rows[0][3].get_str(), Some("unknown"));
    assert_eq!(rows[0][4].get_str(), Some("domain_trust:coding"));
    assert!(
        read_rows(
            temp.path(),
            "?[metric_event_id] := *cognitive_metric_events{metric_event_id, event_kind: 'self_model_prediction_evaluated'}",
        )
        .is_empty(),
        "an unverifiable turn was admitted into the Brier population"
    );
}

/// A domain the self-model has never measured produces no prediction at all,
/// rather than a neutral one that would dilute the calibration population.
#[tokio::test]
async fn a_turn_without_a_self_model_fact_predicts_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with(Arc::new(QuietProvider), temp.path());

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    assert!(
        read_rows(
            temp.path(),
            "?[prediction_id] := *self_model_predictions{prediction_id}",
        )
        .is_empty()
    );
}

/// The emitter the release gate was waiting for: a turn with a deterministic
/// execution failure writes a `self_model_prediction_evaluated` event carrying
/// the pre-action probability.
#[tokio::test]
async fn a_turn_with_a_failing_tool_emits_the_calibration_metric() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with(
        Arc::new(OneFailingToolProvider {
            rounds: AtomicUsize::new(0),
        }),
        temp.path(),
    );
    seed_trust_fact(&agent, "coding", 0.9, 20);

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    let rows = read_rows(
        temp.path(),
        "?[verified_outcome] := *self_model_predictions{verified_outcome}",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0].get_str(), Some("failed"));

    let events = read_rows(
        temp.path(),
        "?[metric_name, value, outcome_status, identities_json] := \
         *cognitive_metric_events{metric_name, value, outcome_status, identities_json, event_kind: 'self_model_prediction_evaluated'}",
    );
    assert_eq!(
        events.len(),
        1,
        "the calibration metric has no live emitter"
    );
    assert_eq!(
        events[0][0].get_str(),
        Some("self_model_confidence_calibration_error")
    );
    assert!((events[0][1].get_float().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(events[0][2].get_str(), Some("failed"));
    let identities = events[0][3].get_str().unwrap();
    assert!(
        identities.contains("\"self_model_backed\":\"true\""),
        "{identities}"
    );
    assert!(identities.contains("\"verification_id\""), "{identities}");
}

// ── #80(b) and #81(a): what the prompt is told ───────────────

/// The startup briefing reports what was measured and names what was not, in
/// the prompt the turn actually built.
#[tokio::test]
async fn the_first_turn_briefs_measured_and_unmeasured_domains() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with(Arc::new(QuietProvider), temp.path());
    seed_trust_fact(&agent, "coding", 0.62, 14);

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    let block = injected_block(&agent).expect("no self-model briefing was injected");
    assert!(block.contains("<self_model_briefing>"), "{block}");
    assert!(
        block.contains("coding: confidence 0.62 over 14 verified outcomes"),
        "{block}"
    );
    assert!(block.contains("No measured evidence yet for:"), "{block}");
    // An unmeasured domain is named, never given a number.
    assert!(!block.contains("git: confidence"), "{block}");
}

/// An unmeasured self-model has nothing to brief, and must not put a block of
/// neutral-looking text into the prompt.
#[tokio::test]
async fn an_unmeasured_self_model_injects_no_briefing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with(Arc::new(QuietProvider), temp.path());

    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");

    assert_eq!(injected_block(&agent), None);
}

/// Issue #81(a): a reflection recorded earlier reaches a later turn, is counted
/// once per turn it is shown on, and stops being shown once its session budget
/// is spent.
#[tokio::test]
async fn an_unresolved_reflection_reaches_later_turns_within_its_budget() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = agent_with(Arc::new(QuietProvider), temp.path());
    seed_triggered_reflection(&agent, "r1");

    for _ in 0..archon_cognitive::MAX_INJECTIONS_PER_REFLECTION {
        agent
            .process_message("fix the failing rust test")
            .await
            .expect("process message");
        let block = injected_block(&agent).expect("no unresolved lesson was injected");
        assert!(
            block.contains("repeated tool failure should stop retrying"),
            "{block}"
        );
        assert!(block.contains("ref:"), "{block}");
    }

    let rows = read_rows(
        temp.path(),
        "?[injection_count, cited_count, verified_reuse_count] := \
         *cognitive_reflection_injections{reflection_id: 'r1', injection_count, cited_count, verified_reuse_count}",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0][0].get_int(),
        Some(archon_cognitive::MAX_INJECTIONS_PER_REFLECTION)
    );
    // The turns never cited it and never verified anything, so neither counter
    // moved: being shown a lesson is not using it.
    assert_eq!(rows[0][1].get_int(), Some(0));
    assert_eq!(rows[0][2].get_int(), Some(0));

    // Budget spent: the next turn is not shown it again.
    agent
        .process_message("fix the failing rust test")
        .await
        .expect("process message");
    assert_eq!(injected_block(&agent), None);
}

// ── citation scanning ────────────────────────────────────────

/// A citation from an earlier turn must not be credited to this one.
#[test]
fn assistant_text_is_bounded_to_the_current_turn() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "earlier turn"}),
        serde_json::json!({"role": "assistant", "content": [
            {"type": "text", "text": "cited ref:deadbeef earlier"}
        ]}),
        serde_json::json!({"role": "user", "content": "this turn"}),
        serde_json::json!({"role": "assistant", "content": [
            {"type": "thinking", "thinking": "ref:cafebabe"},
            {"type": "text", "text": "cited ref:feedface now"}
        ]}),
    ];

    let text = super::turn_assistant_text(&messages, "this turn");

    assert!(text.contains("ref:feedface"));
    assert!(
        !text.contains("ref:deadbeef"),
        "a previous turn's citation leaked in"
    );
    // Thinking blocks are not the assistant's answer and are not scanned.
    assert!(!text.contains("ref:cafebabe"));
}
