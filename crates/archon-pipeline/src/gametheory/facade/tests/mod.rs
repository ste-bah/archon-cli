use super::*;
pub(super) use crate::gametheory::fingerprint::{
    AmbiguityNote, AxisVerdict, GameTheoryFingerprint, HiddenGameDetection,
};
pub(super) use crate::gametheory::routing::RoutingDecision;
use crate::leann_searcher::LeannSearcher;
use crate::runner::{LlmClient, LlmResponse};
use async_trait::async_trait;
use cozo::DbInstance;
use std::sync::Mutex;

mod classification;
mod execution_memory;
mod pipeline;
mod replay_resume;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}
fn test_db() -> DbInstance {
    let path = format!("/tmp/test-gt-facade-{}.db", uuid::Uuid::new_v4());
    DbInstance::new("sqlite", &path, "").unwrap()
}
// ── MockLlmClient for testing LLM integration ─────────────────────────

struct MockLlmClient {
    canned_response: Mutex<String>,
}

impl MockLlmClient {
    fn new(canned: &str) -> Self {
        Self {
            canned_response: Mutex::new(canned.to_string()),
        }
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> std::result::Result<LlmResponse, anyhow::Error> {
        Ok(LlmResponse {
            content: self.canned_response.lock().unwrap().clone(),
            tool_uses: vec![],
            tokens_in: 100,
            tokens_out: 200,
        })
    }
}

struct CapturingLlmClient {
    canned_response: String,
    classification_response: Option<String>,
    prompts: Mutex<Vec<String>>,
    models: Mutex<Vec<String>>,
}

impl CapturingLlmClient {
    fn new(canned: &str) -> Self {
        Self {
            canned_response: canned.to_string(),
            classification_response: None,
            prompts: Mutex::new(Vec::new()),
            models: Mutex::new(Vec::new()),
        }
    }

    fn with_classification(canned: &str, classification: String) -> Self {
        Self {
            canned_response: canned.to_string(),
            classification_response: Some(classification),
            prompts: Mutex::new(Vec::new()),
            models: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }

    fn models(&self) -> Vec<String> {
        self.models.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for CapturingLlmClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        model: &str,
    ) -> std::result::Result<LlmResponse, anyhow::Error> {
        let prompt = messages
            .first()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        self.prompts.lock().unwrap().push(prompt.clone());
        self.models.lock().unwrap().push(model.to_string());
        let content = if prompt.starts_with("Classify this strategic situation") {
            self.classification_response
                .clone()
                .unwrap_or_else(|| self.canned_response.clone())
        } else {
            self.canned_response.clone()
        };
        Ok(LlmResponse {
            content,
            tool_uses: vec![],
            tokens_in: 100,
            tokens_out: 200,
        })
    }
}

struct MockLeannSearcher {
    response: String,
    calls: std::sync::atomic::AtomicUsize,
}

impl MockLeannSearcher {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl LeannSearcher for MockLeannSearcher {
    fn search(&self, _query: &str) -> String {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.response.clone()
    }
}

struct SlowTier1LlmClient {
    response: String,
    active: std::sync::atomic::AtomicUsize,
    max_active: std::sync::atomic::AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

impl SlowTier1LlmClient {
    fn new(response: String) -> Self {
        Self {
            response,
            active: std::sync::atomic::AtomicUsize::new(0),
            max_active: std::sync::atomic::AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn max_active(&self) -> usize {
        self.max_active.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for SlowTier1LlmClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> std::result::Result<LlmResponse, anyhow::Error> {
        let prompt = messages
            .first()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        self.prompts.lock().unwrap().push(prompt);

        let active = self
            .active
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        self.max_active
            .fetch_max(active, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        self.active
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

        Ok(LlmResponse {
            content: self.response.clone(),
            tool_uses: vec![],
            tokens_in: 100,
            tokens_out: 200,
        })
    }
}

fn canned_specialist_llm() -> CapturingLlmClient {
    CapturingLlmClient::new("specialist output")
}

fn canned_fingerprint_json() -> String {
    serde_json::json!({
            "cooperation": {"value": "non-cooperative", "confidence": "high", "rationale": "firms compete"},
            "payoff_sum": {"value": "variable-sum", "confidence": "medium", "rationale": "price outcomes vary"},
            "symmetry": {"value": "asymmetric", "confidence": "medium", "rationale": "costs differ"},
            "timing": {"value": "simultaneous", "confidence": "high", "rationale": "firms act at once"},
            "perfect_info": {"value": "imperfect", "confidence": "medium", "rationale": "prices are not observed before choosing"},
            "complete_info": {"value": "complete", "confidence": "medium", "rationale": "game form is known"},
            "cardinality": {"value": "2-player", "confidence": "high", "rationale": "duopoly"},
            "strategy_space": {"value": "continuous", "confidence": "high", "rationale": "prices or quantities are continuous"},
            "horizon": {"value": "one-shot", "confidence": "medium", "rationale": "single interaction"},
            "primary_family": "Bertrand competition",
            "nearest_classic": "Bertrand duopoly"
        })
        .to_string()
}

fn canned_pipeline_llm() -> CapturingLlmClient {
    CapturingLlmClient::with_classification("specialist output", canned_fingerprint_json())
}

fn test_fingerprint(run_id: &str) -> GameTheoryFingerprint {
    GameTheoryFingerprint {
        run_id: run_id.into(),
        cooperation: AxisVerdict::new("non-cooperative", "high", ""),
        payoff_sum: AxisVerdict::new("zero-sum", "medium", ""),
        symmetry: AxisVerdict::new("asymmetric", "medium", ""),
        timing: AxisVerdict::new("simultaneous", "high", ""),
        perfect_info: AxisVerdict::new("imperfect", "medium", ""),
        complete_info: AxisVerdict::new("complete", "medium", ""),
        cardinality: AxisVerdict::new("2-player", "high", ""),
        strategy_space: AxisVerdict::new("discrete", "medium", ""),
        horizon: AxisVerdict::new("one-shot", "medium", ""),
        primary_family: "test".into(),
        nearest_classic: None,
        shadow_games: vec![],
        hidden_game_scan: None,
        ambiguities: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    }
}
fn seed_kb_pack(db: &DbInstance, pack_id: &str, content: &str) {
    db.run_script(
        ":create doc_sources { document_id: String => source_path: String }",
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        ":create doc_chunks { chunk_id: String => document_id: String, content: String }",
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .unwrap();

    db.run_script(
        "?[document_id, source_path] <- [[$did, $path]] \
             :put doc_sources { document_id => source_path }",
        std::collections::BTreeMap::from([
            ("did".into(), cozo::DataValue::from("doc-policy-pack")),
            (
                "path".into(),
                cozo::DataValue::from(format!("./fixtures/{pack_id}/policy.md")),
            ),
        ]),
        cozo::ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
            "?[chunk_id, document_id, content] <- [[\"chunk-policy-pack-0\", \"doc-policy-pack\", $content]] \
             :put doc_chunks { chunk_id => document_id, content }",
            std::collections::BTreeMap::from([(
                "content".into(),
                cozo::DataValue::from(content),
            )]),
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
}
