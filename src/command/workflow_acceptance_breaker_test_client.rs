//! Deterministic scripted replies using recorded response shapes.
use archon_workflow::llm_client_port::*;
use archon_workflow::{WorkflowLlmClient, WorkflowResult};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
pub(super) fn corpus(name: &str) -> String {
    // Bind scripted calls to immutable recorded attempts, independent of later recordings.
    let name = match name {
        "breaker" => "breaker_attempt1",
        "breaker_gut" => "breaker_attempt1_gut1",
        other => other,
    };
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/breaker_reply_corpus")
            .join(format!("{name}.txt")),
    )
    .unwrap()
}
pub(super) struct Client {
    pub(super) calls: Mutex<Vec<WorkflowAgentCall>>,
    pub(super) completions: Mutex<Vec<String>>,
    pub(super) role_models: BTreeMap<String, String>,
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) replies: Mutex<VecDeque<Result<WorkflowAgentOutcome, String>>>,
    pub(super) fail: bool,
    pub(super) timeout: bool,
    pub(super) judge_fail: bool,
    pub(super) answers: BTreeMap<String, String>,
}
impl Default for Client {
    fn default() -> Self {
        Self {
            calls: Mutex::new(vec![]),
            completions: Mutex::new(vec![]),
            role_models: [
                ("breaker", "author"),
                ("breaker_relevance", "relevance"),
                ("breaker_equivalence", "first"),
                ("breaker_equivalence_second", "second"),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
            aliases: BTreeMap::new(),
            replies: Mutex::new(VecDeque::new()),
            fail: false,
            timeout: false,
            judge_fail: false,
            answers: BTreeMap::new(),
        }
    }
}
impl Client {
    fn outcome(&self, fallback: &str) -> WorkflowResult<WorkflowAgentOutcome> {
        if self.fail {
            return Err(archon_workflow::WorkflowError::port(std::io::Error::other(
                "provider unavailable",
            )));
        }
        let outcome = match self.replies.lock().unwrap().pop_front() {
            Some(Ok(r)) => Ok(r),
            Some(Err(e)) => Err(archon_workflow::WorkflowError::port(std::io::Error::other(
                e,
            ))),
            None => Ok(WorkflowAgentOutcome {
                content: self
                    .answers
                    .get(fallback)
                    .cloned()
                    .unwrap_or_else(|| corpus(fallback)),
                stop_reason: Some("end_turn".into()),
                ..Default::default()
            }),
        };
        outcome.map(|reply| {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/breaker_reply_corpus");
            let literal = !reply.content.is_empty()
                && std::fs::read_dir(dir)
                    .unwrap()
                    .filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
                    .any(|e| {
                        std::fs::read_to_string(e.path())
                            .unwrap()
                            .contains(&reply.content)
                    });
            assert!(
                literal,
                "test reply must be a recorded reply or literal transport fragment"
            );
            reply
        })
    }
}
#[async_trait::async_trait]
impl WorkflowLlmClient for Client {
    fn provider_id(&self) -> Option<String> {
        Some("scripted-fixture".into())
    }
    fn model_for_role(&self, role: &str, default: &str) -> String {
        self.role_models
            .get(role)
            .cloned()
            .unwrap_or_else(|| default.into())
    }
    fn resolve_model_alias(&self, alias: &str) -> String {
        self.aliases
            .get(alias)
            .cloned()
            .unwrap_or_else(|| alias.into())
    }
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.completions.lock().unwrap().push(model.into());
        if self.judge_fail {
            return Err(archon_workflow::WorkflowError::port(std::io::Error::other(
                "judge unavailable",
            )));
        }
        if self.timeout {
            std::future::pending::<()>().await;
        }
        self.outcome(if model == "relevance" {
            "relevant"
        } else {
            "not_equivalent"
        })
    }
    async fn send_message_with_temperature(
        &self,
        m: Vec<serde_json::Value>,
        s: Vec<serde_json::Value>,
        t: Vec<serde_json::Value>,
        model: &str,
        _: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message(m, s, t, model).await
    }
    async fn run_agent(&self, call: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        let gut = call.task.contains("Pick one function or block");
        self.calls.lock().unwrap().push(call);
        if self.timeout {
            std::future::pending::<()>().await;
        }
        self.outcome(if gut { "breaker_gut" } else { "breaker" })
    }
}
