use super::{has_completed_assistant_history, run_subagent_foreground_with_system};
use crate::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use crate::subagent_request::SubagentRequest;
use crate::subagent_session::{CompletedHistory, RuntimeContext, SubagentSession};
use crate::tool::ToolContext;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

struct CountingExecutor(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl SubagentExecutor for CountingExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("unexpected execution".into())
    }

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {}

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, _request: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

#[test]
fn continuation_preflight_requires_the_last_message_to_be_assistant() {
    assert!(!has_completed_assistant_history(&[]));
    assert!(!has_completed_assistant_history(&[
        serde_json::json!({"role":"assistant","content":"answer"}),
        serde_json::json!({"role":"user","content":"repair"}),
    ]));
    assert!(!has_completed_assistant_history(&[
        serde_json::json!({"role":"assistant","content":"answer"}),
        serde_json::json!({"role":"tool","content":"result"}),
    ]));
    assert!(has_completed_assistant_history(&[
        serde_json::json!({"role":"assistant","content":"completed answer"}),
    ]));
}

#[tokio::test]
async fn unusable_repair_records_refusal_without_starting_or_mutating_a_session() {
    let executor_calls = Arc::new(AtomicUsize::new(0));
    install_subagent_executor(Arc::new(CountingExecutor(executor_calls.clone())));
    let history = CompletedHistory::default();
    history.context(Some(RuntimeContext {
        system_prompt: "system".into(),
        model: "model".into(),
        effort: "high".into(),
        critical_system_reminder: Some("reminder".into()),
    }));
    history.append(&serde_json::json!({"role":"user","content":"repair feedback"}));
    let effective_context = history.context(None).unwrap();
    let session = SubagentSession {
        agent_id: "repair-without-answer".into(),
        history: history.clone(),
        continuing: true,
    };
    let request = SubagentRequest {
        prompt: "repair".into(),
        model: None,
        allowed_tools: Vec::new(),
        max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
        timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    };

    let outcome = crate::subagent_session::scope(
        session,
        run_subagent_foreground_with_system(
            "repair-without-answer".into(),
            request,
            Vec::new(),
            CancellationToken::new(),
            ToolContext::default(),
        ),
    )
    .await;

    assert!(matches!(
        outcome,
        crate::subagent_executor::SubagentOutcome::Failed(_)
    ));
    let refusal = history
        .take_refusal()
        .expect("continuation refusal recorded");
    assert!(refusal.contains("start a new agent"), "{refusal}");
    assert_eq!(
        executor_calls.load(Ordering::SeqCst),
        0,
        "manager generation was untouched"
    );
    let after = history.context(None).unwrap();
    assert_eq!(after.system_prompt, effective_context.system_prompt);
    assert_eq!(after.model, effective_context.model);
    assert_eq!(after.effort, effective_context.effort);
    assert_eq!(
        after.critical_system_reminder,
        effective_context.critical_system_reminder
    );
}
