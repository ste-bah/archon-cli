//! A scripted author and judge for re-author tests: no model is called.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentOutcome, WorkflowLlmClient,
};
use async_trait::async_trait;
use serde_json::Value;

type AuthorFn = dyn Fn(&Value, usize) -> String + Send + Sync;
type JudgeFn = dyn Fn(&str, &Value) -> bool + Send + Sync;

/// `author(entry_being_replaced, attempt)` returns the author's reply;
/// `judge(id, check)` says whether the judge accepts that check.
pub(crate) struct ScriptedAuthorJudge {
    author: Box<AuthorFn>,
    judge: Box<JudgeFn>,
    pub(crate) author_calls: AtomicUsize,
    pub(crate) judged_ids: Mutex<Vec<String>>,
    /// The model every judge call asked for.
    pub(crate) judged_models: Mutex<Vec<String>>,
    /// The agent key of every author call, each resolved through the real
    /// agent registry before the scripted reply is returned.
    pub(crate) resolved_agents: Mutex<Vec<String>>,
    /// Every author prompt, in call order: the findings each attempt saw.
    pub(crate) prompts: Mutex<Vec<String>>,
    provider: String,
}

/// The provider the scripted client reports, and fixtures record.
pub(crate) const SCRIPTED_PROVIDER: &str = "scripted";

impl ScriptedAuthorJudge {
    pub(crate) fn new(
        author: impl Fn(&Value, usize) -> String + Send + Sync + 'static,
        judge: impl Fn(&str, &Value) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            author: Box::new(author),
            judge: Box::new(judge),
            author_calls: AtomicUsize::new(0),
            judged_ids: Mutex::new(Vec::new()),
            judged_models: Mutex::new(Vec::new()),
            resolved_agents: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
            provider: SCRIPTED_PROVIDER.into(),
        }
    }

    /// Report `provider` instead of the scripted default.
    pub(crate) fn with_provider(mut self, provider: &str) -> Self {
        self.provider = provider.into();
        self
    }

    pub(crate) fn authored(&self) -> usize {
        self.author_calls.load(Ordering::SeqCst)
    }
}

/// An author reply replacing the entry's command check with `command`.
pub(crate) fn command_entry(entry: &Value, command: &str) -> String {
    serde_json::json!({
        "id": entry["id"],
        "criterion": "",
        "check": {"kind": "command", "command": command, "cwd": "project_root"},
        "gap_permitted": entry["gap_permitted"],
        "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
    })
    .to_string()
}

fn entry_being_replaced(prompt: &str) -> Value {
    let line = prompt
        .lines()
        .find_map(|line| line.strip_prefix("The entry being replaced: "))
        .expect("the author prompt names the entry it replaces");
    serde_json::from_str(line).expect("the replaced entry is JSON")
}

/// Resolve `key` exactly as the subagent executor does (`resolve_agent` in
/// `archon-core` `subagent_executor/run_prepare.rs` reads the registry
/// `AgentRegistry::load` builds), in a project with no agent directories: an
/// author key only some project's `.archon/agents` defines fails here, as it
/// fails the live launch with "Unknown subagent type".
pub(crate) fn resolve_in_bare_project(key: &str) -> archon_core::agents::CustomAgentDefinition {
    let project = tempfile::TempDir::new().expect("temp project");
    archon_core::agents::AgentRegistry::load_with_user_home(project.path(), None)
        .resolve(key)
        .cloned()
        .unwrap_or_else(|| panic!("Unknown subagent type '{key}': the author launch would fail"))
}

fn outcome(content: String) -> WorkflowAgentOutcome {
    WorkflowAgentOutcome {
        content,
        stop_reason: Some("end_turn".into()),
        ..WorkflowAgentOutcome::default()
    }
}

#[async_trait]
impl WorkflowLlmClient for ScriptedAuthorJudge {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    fn provider_id(&self) -> Option<String> {
        Some(self.provider.clone())
    }

    async fn run_agent(&self, call: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        assert!(
            call.allowed_tools
                .iter()
                .all(|tool| tool != "Write" && tool != "Bash"),
            "the re-author may only read"
        );
        let def = resolve_in_bare_project(&call.agent.key);
        assert!(
            def.allowed_tools.as_ref().is_some_and(|tools| tools
                .iter()
                .all(|tool| tool != "Write" && tool != "Edit" && tool != "Bash")),
            "the resolved re-author definition may only read"
        );
        self.resolved_agents.lock().unwrap().push(call.agent.key);
        self.author_calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(call.task.clone());
        let entry = entry_being_replaced(&call.task);
        Ok(outcome((self.author)(&entry, call.attempt)))
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<Value>,
        _system: Vec<Value>,
        _tools: Vec<Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        self.judged_models.lock().unwrap().push(model.to_string());
        let prompt = messages[0]["content"].as_str().expect("judge prompt");
        let checks: Vec<Value> = serde_json::from_str(
            prompt
                .rsplit_once("Checks: ")
                .expect("judge prompt lists its checks")
                .1,
        )
        .expect("checks are JSON");
        let decisions = checks
            .iter()
            .map(|check| {
                let id = check["id"].as_str().expect("id").to_string();
                self.judged_ids.lock().unwrap().push(id.clone());
                let accepted = (self.judge)(&id, &check["check"]);
                serde_json::json!({
                    "id": id,
                    "verdict": if accepted { "accepted" } else { "refuted" },
                    "counterexample": if accepted { "none is constructible" } else { "a passing state where the criterion is false" },
                    "reason": if accepted { "the check fails whenever the criterion is false" } else { "the check misses a branch of the criterion" },
                })
            })
            .collect::<Vec<_>>();
        Ok(outcome(
            serde_json::json!({ "decisions": decisions }).to_string(),
        ))
    }

    async fn send_message(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message_with_temperature(messages, system, tools, model, 0.0)
            .await
    }
}
