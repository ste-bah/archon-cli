use std::path::PathBuf;
use std::sync::Arc;

use archon_llm::provider::LlmProvider;
use tokio::sync::Mutex;

use super::{Subtask, SubtaskExecutor};
use crate::agent::{Agent, AgentConfig, AgentEvent, TimestampedEvent};
use crate::agents::AgentRegistry;
use crate::dispatch::create_default_registry;

/// Production executor that spawns a real Agent per subtask.
pub struct RealSubtaskExecutor {
    provider: Arc<dyn LlmProvider>,
    working_dir: PathBuf,
    model: String,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    pub(super) session_store: Arc<archon_session::storage::SessionStore>,
}

impl RealSubtaskExecutor {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        working_dir: PathBuf,
        model: String,
        agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
        session_store: Arc<archon_session::storage::SessionStore>,
    ) -> Self {
        Self {
            provider,
            working_dir,
            model,
            agent_registry,
            session_store,
        }
    }
}

#[async_trait::async_trait]
impl SubtaskExecutor for RealSubtaskExecutor {
    async fn execute(&self, subtask: &Subtask, context: &str) -> anyhow::Result<String> {
        let prompt = if context.is_empty() {
            subtask.description.clone()
        } else {
            format!(
                "{}\n\nContext from previous tasks:\n{}",
                subtask.description, context
            )
        };
        let registry = create_default_registry(self.working_dir.clone(), None);
        let tool_defs = registry.tool_definitions();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<TimestampedEvent>(
            crate::agent::AGENT_EVENT_CHANNEL_CAPACITY,
        );

        let session_id = format!("team:{}:{}", subtask.agent_type, uuid::Uuid::new_v4());
        self.session_store
            .register_session(
                &session_id,
                &self.working_dir.display().to_string(),
                None,
                &self.model,
            )
            .map_err(|error| anyhow::anyhow!("team session registration failed: {error}"))?;
        let config = AgentConfig {
            model: self.model.clone(),
            session_id,
            system_prompt: vec![serde_json::json!({
                "type": "text",
                "text": format!(
                    "You are a {} agent. Complete the assigned task concisely and return the result.",
                    subtask.agent_type
                ),
            })],
            tools: tool_defs,
            working_dir: self.working_dir.clone(),
            agent_type: subtask.agent_type.clone(),
            permission_mode: Arc::new(Mutex::new("bypassPermissions".to_string())),
            ..AgentConfig::default()
        };
        let mut agent = Agent::new(
            self.provider.clone(),
            registry,
            config,
            event_tx,
            self.agent_registry.clone(),
        );
        agent.set_session_store(Arc::clone(&self.session_store));
        agent.install_subagent_executor();

        let output = Arc::new(Mutex::new(String::new()));
        let output_collector = Arc::clone(&output);
        let collector_handle = tokio::spawn(async move {
            while let Some(ts) = event_rx.recv().await {
                if let AgentEvent::TextDelta(text) = ts.inner {
                    output_collector.lock().await.push_str(&text);
                }
            }
        });
        agent
            .process_message(&prompt)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        drop(agent);
        let _ = collector_handle.await;

        let result = output.lock().await.clone();
        if result.is_empty() {
            Ok(format!(
                "[{}: completed with no text output]",
                subtask.agent_type
            ))
        } else {
            Ok(result)
        }
    }
}
