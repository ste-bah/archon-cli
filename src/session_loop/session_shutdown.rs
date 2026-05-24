use std::sync::Arc;

use archon_core::agent::Agent;

pub(super) async fn finish_session(
    agent_def: &Option<archon_core::agents::CustomAgentDefinition>,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
) {
    increment_agent_invocation(agent_def);
    drain_inflight_turns(dispatcher).await;
    flush_auto_extractions(agent).await;
    fire_stop_hooks(agent).await;
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

async fn drain_inflight_turns(dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>) {
    let drain_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while (dispatcher.lock().unwrap().is_busy() || dispatcher.lock().unwrap().queue_len() > 0)
        && std::time::Instant::now() < drain_deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        let _ = dispatcher.lock().unwrap().poll_completion();
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
