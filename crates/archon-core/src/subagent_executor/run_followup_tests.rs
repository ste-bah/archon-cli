use super::tests::{queued_request, test_executor_with_sink};
use super::*;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_observability::{AgentActivityStatus as Status, InMemoryActivitySink};

struct Panics(u8);
#[async_trait::async_trait]
impl LlmProvider for Panics {
    fn name(&self) -> &str {
        "panic-test"
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
        match self.0 {
            0 => panic!("runner literal panic"),
            1 => std::panic::panic_any(String::from("runner owned panic")),
            _ => std::panic::panic_any(37u32),
        }
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
}

async fn panic_case(payload: u8, expected: &str) {
    let sink = InMemoryActivitySink::new();
    let mut executor = test_executor_with_sink(1, Some(Arc::new(sink.clone())));
    Arc::get_mut(&mut executor).unwrap().client = Arc::new(Panics(payload));
    let task = tokio::spawn(async move {
        executor
            .run_subagent_to_completion(
                "panic-run".into(),
                queued_request(),
                ToolContext::default(),
                CancellationToken::new(),
            )
            .await
    });
    let result = task.await.expect("runner panic is contained");
    assert!(result.unwrap_err().to_string().contains(expected));
    let events = sink.events();
    let last = events.last().unwrap();
    assert_eq!(last.status, Status::Failed);
    assert!(last.message.contains(expected), "{}", last.message);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(
                e.status,
                Status::Failed | Status::Cancelled | Status::Completed
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn gc_runner_literal_panic_is_failed() {
    panic_case(0, "runner literal panic").await;
}
#[tokio::test]
async fn gc_runner_owned_panic_is_failed() {
    panic_case(1, "runner owned panic").await;
}
#[tokio::test]
async fn gc_runner_opaque_panic_is_failed() {
    panic_case(2, "non-string panic payload").await;
}

fn distinct_calls(status: Status) {
    let sink = InMemoryActivitySink::new();
    let executor = test_executor_with_sink(1, Some(Arc::new(sink.clone())));
    let mut live = executor.activity_row("shared");
    let mut refused = executor.activity_row("shared");
    live.started("worker", "m");
    refused.queued("worker", "m", "waiting".into());
    if status == Status::Failed {
        refused.settle(
            &Err(ExecutorError::Internal("AlreadyRunning".into())),
            &CancellationToken::new(),
        );
    } else if status == Status::Completed {
        refused.finished("worker", "m", &Ok("done".into()));
    } else {
        drop(refused);
    }
    let events = sink.events();
    assert!(events[0].agent_id.is_some(), "call instance missing");
    assert_ne!(events[0].agent_id, events[1].agent_id);
    assert_eq!(events[1].agent_id, events[2].agent_id);
    assert_eq!(events[2].status, status);
}
#[test]
fn gc_duplicate_refusal_has_its_own_row() {
    distinct_calls(Status::Failed);
}
#[test]
fn gc_duplicate_completion_has_its_own_row() {
    distinct_calls(Status::Completed);
}
#[test]
fn gc_duplicate_drop_has_its_own_row() {
    distinct_calls(Status::Cancelled);
}
