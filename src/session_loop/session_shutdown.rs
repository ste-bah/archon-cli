use std::sync::Arc;

use archon_core::agent::Agent;

#[cfg(not(test))]
const TURN_ABORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(test)]
const TURN_ABORT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(10);

pub(super) async fn finish_session(
    agent_def: &Option<archon_core::agents::CustomAgentDefinition>,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    sandbox_audit_drain: crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle,
) -> anyhow::Result<()> {
    increment_agent_invocation(agent_def);
    close_terminals(agent).await;
    let turn_result = drain_inflight_turns(dispatcher).await;
    if turn_result.is_ok() {
        flush_auto_extractions(agent).await;
        fire_stop_hooks(agent).await;
    }
    let audit_result = drain_sandbox_audit(&sandbox_audit_drain).await;
    finish_turn_and_audit(turn_result, audit_result)
}

fn finish_turn_and_audit(
    turn_result: anyhow::Result<()>,
    audit_result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match (turn_result, audit_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(turn_error), Ok(())) => Err(turn_error),
        (Ok(()), Err(audit_error)) => Err(audit_error),
        (Err(turn_error), Err(audit_error)) => Err(anyhow::anyhow!(
            "session turn shutdown failed: {turn_error:#}; sandbox audit drain failed: {audit_error:#}"
        )),
    }
}

/// Kill the persistent shells this session opened (#189 Phase 6).
///
/// First, before anything that can fail or block: a terminal is a live process,
/// and the one outcome that is not acceptable is leaving one running because
/// shutdown took a different path out. The cap and the idle timeout both leave
/// a recently-used terminal alone, which is exactly the state one is in here.
async fn close_terminals(agent: &Arc<tokio::sync::Mutex<Agent>>) {
    let session_id = agent.lock().await.session_id().to_string();
    let closed = archon_tools::terminal_tools::close_session_terminals(&session_id);
    if closed > 0 {
        tracing::info!(closed, %session_id, "closed persistent terminals on session end");
    }
}

fn increment_agent_invocation(agent_def: &Option<archon_core::agents::CustomAgentDefinition>) {
    if let Some(def) = agent_def
        && let Some(ref base_dir) = def.base_dir
    {
        let agent_dir = std::path::Path::new(base_dir);
        if let Err(error) = archon_core::agents::memory::increment_invocation_count(agent_dir) {
            tracing::warn!(
                agent = def.agent_type.as_str(),
                "failed to increment invocation count: {error}"
            );
        }
    }
}

async fn drain_inflight_turns(
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
) -> anyhow::Result<()> {
    let task = {
        let mut dispatcher = dispatcher.lock().unwrap();
        dispatcher.pending_queue.clear();
        dispatcher.current_query.take()
    };
    let Some(mut task) = task else {
        return Ok(());
    };
    task.abort();
    match tokio::time::timeout(TURN_ABORT_TIMEOUT, &mut task).await {
        Ok(_) => Ok(()),
        Err(_) => anyhow::bail!(
            "session turn remained active {TURN_ABORT_TIMEOUT:?} after shutdown abort"
        ),
    }
}

async fn flush_auto_extractions(agent: &Arc<tokio::sync::Mutex<Agent>>) {
    let flushed = agent
        .lock()
        .await
        .flush_auto_extractions(std::time::Duration::from_secs(10))
        .await;
    if flushed > 0 {
        tracing::info!(
            count = flushed,
            "flushed pending auto-extraction tasks before session shutdown"
        );
    }
}

async fn drain_sandbox_audit(
    drain: &crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle,
) -> anyhow::Result<()> {
    if let Some(readback) = drain.shutdown(std::time::Duration::from_secs(30)).await? {
        tracing::info!(
            accepted = readback.accepted,
            dropped = readback.dropped,
            persisted = readback.persisted,
            failed = readback.failed,
            "sandbox audit writer drained"
        );
    }
    Ok(())
}

async fn fire_stop_hooks(agent: &Arc<tokio::sync::Mutex<Agent>>) {
    let stop_fut = {
        let guard = agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::Stop,
            serde_json::json!({
                "hook_event": "Stop",
                "reason": "session_end",
            }),
        )
    };
    let stop_result = tokio::time::timeout(std::time::Duration::from_secs(10), stop_fut).await;
    if stop_result.is_err() {
        tracing::warn!("Stop hook timed out — firing StopFailure");
        {
            let guard = agent.lock().await;
            guard.fire_hook_detached(
                archon_core::hooks::HookType::StopFailure,
                serde_json::json!({
                    "hook_event": "StopFailure",
                    "reason": "stop_hook_timeout",
                }),
            )
        }
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopTurnRunner;

    impl archon_tui::TurnRunner for NoopTurnRunner {
        fn run_turn<'a>(
            &'a self,
            _prompt: String,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>>
        {
            Box::pin(async { Ok(()) })
        }
    }

    fn dispatcher() -> Arc<std::sync::Mutex<archon_tui::AgentDispatcher>> {
        Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::channel(1).0,
        )))
    }

    #[tokio::test]
    async fn turn_drain_discards_queued_prompts_before_cancellation() {
        let dispatcher = dispatcher();
        {
            let mut dispatcher = dispatcher.lock().unwrap();
            dispatcher.current_query =
                Some(tokio::spawn(std::future::pending::<anyhow::Result<()>>()));
            dispatcher
                .pending_queue
                .push_back(archon_tui::QueuedPrompt {
                    prompt: "must not run".to_string(),
                    agent_id: None,
                    submitted_at: std::time::Instant::now(),
                    runner: Arc::new(NoopTurnRunner),
                });
        }

        drain_inflight_turns(&dispatcher).await.unwrap();

        let dispatcher = dispatcher.lock().unwrap();
        assert_eq!(dispatcher.queue_len(), 0);
        assert!(!dispatcher.is_busy());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn turn_drain_cancels_and_bounds_abort_resistant_task() {
        let dispatcher = dispatcher();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        dispatcher.lock().unwrap().current_query = Some(tokio::task::spawn_blocking(move || {
            let _ = started_tx.send(());
            std::thread::sleep(std::time::Duration::from_millis(200));
            Ok(())
        }));
        started_rx.await.unwrap();

        let error = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            drain_inflight_turns(&dispatcher),
        )
        .await
        .expect("turn drain must remain bounded")
        .expect_err("stalled turn drain must fail loud");

        assert!(error.to_string().contains("remained active"), "{error:#}");
        assert!(!dispatcher.lock().unwrap().is_busy());
    }

    #[test]
    fn turn_and_audit_failures_remain_visible_together() {
        let error = finish_turn_and_audit(
            Err(anyhow::anyhow!("turn failed")),
            Err(anyhow::anyhow!("audit failed")),
        )
        .expect_err("both shutdown failures must remain visible");
        let message = error.to_string();

        assert!(message.contains("turn failed"), "{error:#}");
        assert!(message.contains("audit failed"), "{error:#}");
    }
}
