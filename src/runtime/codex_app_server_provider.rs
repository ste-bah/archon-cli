//! Codex app-server provider adapter.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Result;
use archon_core::config::CodexProviderConfig;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::runtime::codex_app_server_rpc::{CodexAppServerRpcClient, CodexNotification};

pub(crate) struct CodexAppServerProvider {
    config: CodexProviderConfig,
    model_cache: crate::runtime::codex_app_server_models::ModelCache,
}

impl CodexAppServerProvider {
    pub(crate) fn new(config: CodexProviderConfig) -> Result<Self> {
        let discovery = crate::runtime::codex_app_server::discover_codex_app_server(&config);
        if !discovery.is_configured() {
            anyhow::bail!(
                "Codex app-server target is not configured: {}",
                discovery.reason_code()
            );
        }
        let model_cache = Arc::new(RwLock::new(
            crate::runtime::codex_app_server_models::fallback_models(&config),
        ));
        Ok(Self {
            config,
            model_cache,
        })
    }
}

#[async_trait]
impl LlmProvider for CodexAppServerProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.model_cache
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    async fn stream(&self, request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        if !request.tools.is_empty() {
            return Err(LlmError::Unsupported(
                "Codex app-server mode cannot execute Archon-managed tool calls directly; use runtime=auto with direct_fallback=true or runtime=direct for governed tool use".into(),
            ));
        }
        let config = self.config.clone();
        let model_cache = Arc::clone(&self.model_cache);
        let timeout_ms = config.app_server_discovery_timeout_ms.max(100);
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            if let Err(error) =
                run_app_server_turn(config, request, tx.clone(), timeout_ms, model_cache).await
            {
                let _ = tx
                    .send(StreamEvent::Error {
                        error_type: "codex_app_server_error".into(),
                        message: error.to_string(),
                    })
                    .await;
            }
        });
        Ok(rx)
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut rx = self.stream(request).await?;
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut stop_reason = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::MessageStart { usage: start, .. } => usage.merge(&start),
                StreamEvent::TextDelta { text: delta, .. } => text.push_str(&delta),
                StreamEvent::MessageDelta {
                    usage: delta_usage,
                    stop_reason: delta_stop,
                } => {
                    if let Some(delta_usage) = delta_usage {
                        usage.merge(&delta_usage);
                    }
                    if let Some(delta_stop) = delta_stop {
                        stop_reason = delta_stop;
                    }
                }
                StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    return Err(archon_llm::context_window::classify_context_window_error(
                        None,
                        Some(&error_type),
                        None,
                        &message,
                        Some("codex-app-server"),
                        None,
                    )
                    .unwrap_or(LlmError::Http(message)));
                }
                _ => {}
            }
        }
        Ok(LlmResponse {
            content: if text.is_empty() {
                Vec::new()
            } else {
                vec![serde_json::json!({"type": "text", "text": text})]
            },
            usage,
            stop_reason,
        })
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Streaming
                | ProviderFeature::Thinking
                | ProviderFeature::Vision
                | ProviderFeature::SystemPrompt
        )
    }
}

async fn run_app_server_turn(
    config: CodexProviderConfig,
    request: LlmRequest,
    tx: mpsc::Sender<StreamEvent>,
    timeout_ms: u64,
    model_cache: crate::runtime::codex_app_server_models::ModelCache,
) -> Result<(), LlmError> {
    let (client, mut notifications) = CodexAppServerRpcClient::connect(&config).await?;
    client.initialize(timeout_ms).await?;
    crate::runtime::codex_app_server_models::refresh_model_cache(&client, timeout_ms, &model_cache)
        .await;
    let cwd = std::env::current_dir()
        .map_err(|e| LlmError::Http(e.to_string()))?
        .display()
        .to_string();
    let thread = client
        .request(
            "thread/start",
            thread_start_params(&request, &cwd),
            timeout_ms,
        )
        .await?;
    let thread_id = read_nested_string(&thread, &["thread", "id"])
        .ok_or_else(|| LlmError::Serialize("thread/start response missing thread.id".into()))?;
    let turn = client
        .request(
            "turn/start",
            turn_start_params(&request, &thread_id, &cwd),
            timeout_ms,
        )
        .await?;
    let turn_id = read_nested_string(&turn, &["turn", "id"])
        .ok_or_else(|| LlmError::Serialize("turn/start response missing turn.id".into()))?;
    tx.send(StreamEvent::MessageStart {
        id: turn_id.clone(),
        model: request.model.clone(),
        usage: Usage::default(),
    })
    .await
    .ok();

    let mut projector =
        AppServerStreamProjector::new(thread_id, turn_id, request.model.clone(), tx);
    projector.project_turn_snapshot(turn.get("turn")).await;
    let idle = Duration::from_millis(timeout_ms.max(100));
    while !projector.completed {
        match tokio::time::timeout(idle, notifications.recv()).await {
            Ok(Some(notification)) => projector.handle_notification(notification).await,
            Ok(None) => break,
            Err(_) => {
                projector
                    .send_error("timeout", "Codex app-server turn timed out")
                    .await;
                break;
            }
        }
    }
    Ok(())
}

struct AppServerStreamProjector {
    thread_id: String,
    turn_id: String,
    model_id: String,
    tx: mpsc::Sender<StreamEvent>,
    text_started: bool,
    emitted_text: bool,
    completed: bool,
}

impl AppServerStreamProjector {
    fn new(
        thread_id: String,
        turn_id: String,
        model_id: String,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Self {
        Self {
            thread_id,
            turn_id,
            model_id,
            tx,
            text_started: false,
            emitted_text: false,
            completed: false,
        }
    }

    async fn handle_notification(&mut self, notification: CodexNotification) {
        if notification.method == "account/rateLimits/updated" {
            super::codex_app_server_limits::record_rate_limits(
                &notification.params,
                Some(&self.model_id),
            );
            return;
        }
        if !self.is_current_turn(&notification.params) {
            return;
        }
        match notification.method.as_str() {
            "item/agentMessage/delta" => {
                if let Some(delta) = read_string(&notification.params, "delta") {
                    self.send_text_delta(delta).await;
                }
            }
            "item/completed" => {
                if let Some(item) = notification.params.get("item") {
                    self.project_completed_item(item).await;
                }
            }
            "turn/completed" => {
                self.project_turn_snapshot(notification.params.get("turn"))
                    .await;
                self.finish_turn(notification.params.get("turn")).await;
            }
            _ => {}
        }
    }

    async fn project_turn_snapshot(&mut self, turn: Option<&Value>) {
        let Some(items) = turn
            .and_then(|value| value.get("items"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for item in items {
            self.project_completed_item(item).await;
        }
    }

    async fn project_completed_item(&mut self, item: &Value) {
        if read_string(item, "type") == Some("agentMessage")
            && let Some(text) = read_string(item, "text")
            && !text.trim().is_empty()
            && !self.emitted_text
        {
            self.send_text_delta(text).await;
        }
    }

    async fn finish_turn(&mut self, turn: Option<&Value>) {
        if let Some(status) = turn.and_then(|value| read_string(value, "status"))
            && status == "failed"
        {
            let message = turn
                .and_then(|value| value.pointer("/error/message"))
                .and_then(Value::as_str)
                .unwrap_or("Codex app-server turn failed");
            self.send_error("turn_failed", message).await;
        }
        if self.text_started {
            self.tx
                .send(StreamEvent::ContentBlockStop { index: 0 })
                .await
                .ok();
        }
        self.tx
            .send(StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: None,
            })
            .await
            .ok();
        self.tx.send(StreamEvent::MessageStop).await.ok();
        self.completed = true;
    }

    async fn send_text_delta(&mut self, text: &str) {
        if !self.text_started {
            self.tx
                .send(StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                })
                .await
                .ok();
            self.text_started = true;
        }
        self.emitted_text = true;
        self.tx
            .send(StreamEvent::TextDelta {
                index: 0,
                text: text.to_string(),
            })
            .await
            .ok();
    }

    async fn send_error(&mut self, error_type: &str, message: &str) {
        self.tx
            .send(StreamEvent::Error {
                error_type: error_type.into(),
                message: message.into(),
            })
            .await
            .ok();
        self.completed = true;
    }

    fn is_current_turn(&self, params: &Value) -> bool {
        let thread_matches = read_string(params, "threadId")
            .map(|id| id == self.thread_id)
            .unwrap_or(true);
        let turn_matches = read_string(params, "turnId")
            .map(|id| id == self.turn_id)
            .unwrap_or(true);
        thread_matches && turn_matches
    }
}

fn thread_start_params(request: &LlmRequest, cwd: &str) -> Value {
    serde_json::json!({
        "model": request.model.clone(),
        "cwd": cwd,
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "serviceName": "Archon",
        "developerInstructions": system_text(&request.system),
        "dynamicTools": [],
        "experimentalRawEvents": true,
        "persistExtendedHistory": false,
    })
}

fn turn_start_params(request: &LlmRequest, thread_id: &str, cwd: &str) -> Value {
    serde_json::json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": messages_text(&request.messages), "text_elements": []}],
        "cwd": cwd,
        "approvalPolicy": "never",
        "sandboxPolicy": "read-only",
        "model": request.model.clone(),
        "effort": request.effort.clone(),
    })
}

fn system_text(system: &[Value]) -> String {
    system
        .iter()
        .filter_map(extract_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn messages_text(messages: &[Value]) -> String {
    messages
        .iter()
        .filter_map(|message| {
            let role = read_string(message, "role").unwrap_or("message");
            let text = message.get("content").and_then(extract_text)?;
            Some(format!("{role}:\n{text}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(extract_text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| map.get("content").and_then(extract_text)),
        _ => None,
    }
}

fn read_nested_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for part in path {
        current = current.get(*part)?;
    }
    current.as_str().map(str::to_string)
}

fn read_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_provider() -> CodexAppServerProvider {
        CodexAppServerProvider::new(CodexProviderConfig {
            app_server_transport: "stdio".into(),
            app_server_command: "codex".into(),
            ..CodexProviderConfig::default()
        })
        .unwrap()
    }

    #[tokio::test]
    async fn provider_rejects_tools_without_direct_fallback() {
        let provider = configured_provider();
        let request = LlmRequest {
            tools: vec![serde_json::json!({
                "name": "Bash",
                "description": "run command",
                "input_schema": {"type": "object"}
            })],
            ..LlmRequest::default()
        };

        let error = provider.stream(request).await.unwrap_err().to_string();

        assert!(error.contains("cannot execute Archon-managed tool calls directly"));
    }

    #[tokio::test]
    async fn projector_emits_completed_turn_text() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut projector =
            AppServerStreamProjector::new("thread-1".into(), "turn-1".into(), "gpt-5.4".into(), tx);
        projector
            .handle_notification(CodexNotification {
                method: "turn/completed".into(),
                params: serde_json::json!({
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "turn": {
                        "id": "turn-1",
                        "threadId": "thread-1",
                        "status": "completed",
                        "items": [{
                            "id": "item-1",
                            "type": "agentMessage",
                            "text": "done"
                        }]
                    }
                }),
            })
            .await;

        let mut saw_text = false;
        let mut saw_stop = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                StreamEvent::TextDelta { text, .. } if text == "done" => saw_text = true,
                StreamEvent::MessageStop => saw_stop = true,
                _ => {}
            }
        }

        assert!(saw_text);
        assert!(saw_stop);
        assert!(projector.completed);
    }
}
