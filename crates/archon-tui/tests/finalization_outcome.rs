use std::sync::Arc;

use archon_core::agent::{AgentLoopError, TimestampedEvent};
use archon_tui::{AgentDispatcher, AgentRouter, TurnOutcome, TurnRunner};

struct NoopRouter;

impl AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

struct FinalizationBlockedRunner;

impl TurnRunner for FinalizationBlockedRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async {
            Err(anyhow::Error::new(AgentLoopError::FinalizationBlocked(
                "run tests".into(),
            )))
        })
    }
}

#[tokio::test]
async fn dispatcher_preserves_finalization_blocked_outcome() {
    let (tx, _rx) = tokio::sync::mpsc::channel::<TimestampedEvent>(
        archon_core::agent::AGENT_EVENT_CHANNEL_CAPACITY,
    );
    let mut dispatcher = AgentDispatcher::new(Arc::new(NoopRouter), tx);
    dispatcher.spawn_turn("prompt".into(), Arc::new(FinalizationBlockedRunner));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let outcome = loop {
        if let Some(outcome) = dispatcher.poll_completion() {
            break outcome;
        }
        assert!(std::time::Instant::now() < deadline, "turn did not finish");
        tokio::task::yield_now().await;
    };

    assert!(matches!(
        outcome,
        TurnOutcome::FinalizationBlocked(message) if message == "run tests"
    ));
}
