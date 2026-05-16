//! Provider-neutral semantic labeler for world-model rows.

use std::collections::VecDeque;

use anyhow::{Result, bail};
use archon_llm::provider::{LlmProvider, LlmRequest};
use serde::{Deserialize, Serialize};

use crate::labels::DeterministicLabelBuilder;
use crate::schema::{WorldLabelSet, WorldTraceRow};

const EXCERPT_PROMPT_LIMITS: [usize; 7] = [2048, 1024, 512, 256, 128, 64, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelerMode {
    Heuristic,
    Llm,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelerOptions {
    pub mode: LabelerMode,
    pub model: String,
    pub max_events_per_prompt: usize,
    pub max_prompt_chars: usize,
}

impl Default for LabelerOptions {
    fn default() -> Self {
        Self {
            mode: LabelerMode::Hybrid,
            model: "configured-provider-default".into(),
            max_events_per_prompt: 30,
            max_prompt_chars: 128_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowLabelUpdate {
    pub row_id: String,
    pub labels: WorldLabelSet,
}

#[derive(Debug, Deserialize)]
struct LlmLabelResponse {
    rows: Vec<RowLabelUpdate>,
}

pub fn heuristic_label_rows(rows: &[WorldTraceRow]) -> Vec<RowLabelUpdate> {
    rows.iter()
        .map(|row| RowLabelUpdate {
            row_id: row.row_id.clone(),
            labels: DeterministicLabelBuilder.label_row(row),
        })
        .collect()
}

pub async fn label_rows_with_provider(
    rows: &[WorldTraceRow],
    options: &LabelerOptions,
    provider: Option<&dyn LlmProvider>,
) -> Result<Vec<RowLabelUpdate>> {
    match options.mode {
        LabelerMode::Heuristic => Ok(heuristic_label_rows(rows)),
        LabelerMode::Llm => {
            llm_label_rows_chunked(
                rows,
                options,
                provider.ok_or_else(|| {
                    anyhow::anyhow!("LLM labeler requested but no LlmProvider was supplied")
                })?,
            )
            .await
        }
        LabelerMode::Hybrid => {
            let mut labels = heuristic_label_rows(rows);
            if let Some(provider) = provider {
                merge_llm_labels(
                    &mut labels,
                    llm_label_rows_chunked(rows, options, provider).await?,
                );
            }
            Ok(labels)
        }
    }
}

async fn llm_label_rows_chunked(
    rows: &[WorldTraceRow],
    options: &LabelerOptions,
    provider: &dyn LlmProvider,
) -> Result<Vec<RowLabelUpdate>> {
    let chunk_size = options.max_events_per_prompt.max(1);
    let mut labels = Vec::new();
    for chunk in rows.chunks(chunk_size) {
        for prompt in label_prompts(chunk, options)? {
            labels.extend(llm_label_prompt(prompt, options, provider).await?);
        }
    }
    Ok(labels)
}

async fn llm_label_prompt(
    prompt: String,
    options: &LabelerOptions,
    provider: &dyn LlmProvider,
) -> Result<Vec<RowLabelUpdate>> {
    let response = provider
        .complete(LlmRequest {
            model: options.model.clone(),
            max_tokens: 8192,
            system: vec![serde_json::json!({
                "type": "text",
                "text": "You label Archon trace rows. Return compact JSON only."
            })],
            messages: vec![serde_json::json!({
                "role": "user",
                "content": prompt
            })],
            request_origin: Some("world_model_labeler".into()),
            ..LlmRequest::default()
        })
        .await
        .map_err(|error| anyhow::anyhow!("world-model LLM labeler failed: {error}"))?;
    parse_label_response(&response.content_text())
}

fn label_prompts(rows: &[WorldTraceRow], options: &LabelerOptions) -> Result<Vec<String>> {
    let mut prompts = Vec::new();
    let mut pending = VecDeque::from([rows]);
    while let Some(chunk) = pending.pop_front() {
        if chunk.is_empty() {
            continue;
        }
        if let Some(prompt) = fitting_label_prompt(chunk, options) {
            prompts.push(prompt);
            continue;
        }
        if chunk.len() == 1 {
            bail!(
                "world-model label prompt exceeds max_prompt_chars even after excerpt truncation: row_id={}",
                chunk[0].row_id
            );
        }
        let mid = chunk.len() / 2;
        let (left, right) = chunk.split_at(mid);
        pending.push_front(right);
        pending.push_front(left);
    }
    Ok(prompts)
}

fn fitting_label_prompt(rows: &[WorldTraceRow], options: &LabelerOptions) -> Option<String> {
    EXCERPT_PROMPT_LIMITS
        .iter()
        .filter_map(|limit| {
            let prompt = label_prompt_with_excerpt_limit(rows, options, *limit);
            (prompt.len() <= options.max_prompt_chars).then_some(prompt)
        })
        .next()
}

fn label_prompt_with_excerpt_limit(
    rows: &[WorldTraceRow],
    options: &LabelerOptions,
    excerpt_limit: usize,
) -> String {
    let mut events = Vec::new();
    for row in rows.iter().take(options.max_events_per_prompt) {
        let (excerpt, excerpt_truncated, excerpt_chars) =
            prompt_excerpt(row.redacted_excerpt.as_deref(), excerpt_limit);
        events.push(serde_json::json!({
            "row_id": row.row_id,
            "source": row.source,
            "action_kind": row.action_kind,
            "provider": row.provider,
            "model": row.model,
            "agent": row.agent,
            "excerpt": excerpt,
            "excerpt_truncated": excerpt_truncated,
            "excerpt_chars": excerpt_chars,
            "labels": row.labels,
            "scalar": row.scalar_features,
        }));
    }
    serde_json::json!({
        "task": "Return labels for each row. Preserve row_id. Use booleans for failure, retry, provider_incident, verification_needed, user_correction, plan_drift, high_cost, slow_run and optional success.",
        "schema": {"rows": [{"row_id": "string", "labels": {}}]},
        "rows": events,
    })
    .to_string()
}

fn prompt_excerpt(text: Option<&str>, excerpt_limit: usize) -> (Option<String>, bool, usize) {
    let Some(text) = text else {
        return (None, false, 0);
    };
    let char_count = text.chars().count();
    if char_count <= excerpt_limit {
        return (Some(text.to_string()), false, char_count);
    }
    (
        Some(text.chars().take(excerpt_limit).collect()),
        true,
        char_count,
    )
}

fn parse_label_response(text: &str) -> Result<Vec<RowLabelUpdate>> {
    let trimmed = strip_json_fence(text.trim());
    let response: LlmLabelResponse = serde_json::from_str(trimmed)
        .or_else(|_| extract_json_object(trimmed).and_then(serde_json::from_str))
        .map_err(|error| anyhow::anyhow!("invalid world-model label JSON: {error}"))?;
    Ok(response.rows)
}

fn strip_json_fence(text: &str) -> &str {
    let Some(stripped) = text.strip_prefix("```") else {
        return text;
    };
    let after_language = stripped
        .strip_prefix("json")
        .or_else(|| stripped.strip_prefix("JSON"))
        .unwrap_or(stripped)
        .trim_start_matches('\n')
        .trim();
    after_language
        .strip_suffix("```")
        .unwrap_or(after_language)
        .trim()
}

fn extract_json_object(text: &str) -> serde_json::Result<&str> {
    let Some(start) = text.find('{') else {
        return serde_json::from_str::<serde_json::Value>(text).map(|_| text);
    };
    let Some(end) = text.rfind('}') else {
        return serde_json::from_str::<serde_json::Value>(text).map(|_| text);
    };
    Ok(&text[start..=end])
}

fn merge_llm_labels(base: &mut [RowLabelUpdate], llm: Vec<RowLabelUpdate>) {
    for llm_row in llm {
        if let Some(base_row) = base.iter_mut().find(|row| row.row_id == llm_row.row_id) {
            base_row.labels = llm_row.labels;
        }
    }
}

trait LlmResponseText {
    fn content_text(&self) -> String;
}

impl LlmResponseText for archon_llm::provider::LlmResponse {
    fn content_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{WorldActionKind, WorldTraceRow};
    use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
    use archon_llm::types::Usage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeProvider;

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake"
        }
        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError>
        {
            Err(LlmError::Unsupported("stream".into()))
        }
        async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: vec![
                    serde_json::json!({"text": r#"{"rows":[{"row_id":"r1","labels":{"failure":true,"retry":false,"provider_incident":false,"verification_needed":true,"user_correction":false,"plan_drift":false,"high_cost":false,"slow_run":false}}]}"#}),
                ],
                usage: Usage::default(),
                stop_reason: "stop".into(),
            })
        }
        fn supports_feature(&self, _: ProviderFeature) -> bool {
            true
        }
    }

    #[derive(Default)]
    struct ChunkingProvider {
        calls: AtomicUsize,
        max_prompt_len: AtomicUsize,
        truncated_rows: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl LlmProvider for ChunkingProvider {
        fn name(&self) -> &str {
            "chunking"
        }
        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError>
        {
            Err(LlmError::Unsupported("stream".into()))
        }
        async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let prompt = request
                .messages
                .first()
                .and_then(|message| message.get("content"))
                .and_then(|content| content.as_str())
                .expect("label prompt content");
            self.max_prompt_len
                .fetch_max(prompt.len(), Ordering::SeqCst);
            let prompt_json: serde_json::Value =
                serde_json::from_str(prompt).expect("label prompt json");
            let rows = prompt_json
                .get("rows")
                .and_then(|rows| rows.as_array())
                .expect("prompt rows");
            self.truncated_rows.fetch_add(
                rows.iter()
                    .filter(|row| {
                        row.get("excerpt_truncated")
                            .and_then(|value| value.as_bool())
                            == Some(true)
                    })
                    .count(),
                Ordering::SeqCst,
            );
            let labels = rows
                .iter()
                .map(|row| {
                    let row_id = row
                        .get("row_id")
                        .and_then(|value| value.as_str())
                        .expect("row_id");
                    serde_json::json!({"row_id": row_id, "labels": {"retry": true}})
                })
                .collect::<Vec<_>>();
            Ok(LlmResponse {
                content: vec![
                    serde_json::json!({"text": serde_json::json!({"rows": labels}).to_string()}),
                ],
                usage: Usage::default(),
                stop_reason: "stop".into(),
            })
        }
        fn supports_feature(&self, _: ProviderFeature) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn hybrid_labeler_uses_provider_json_when_available() {
        let mut row = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r1");
        row.redacted_excerpt = Some("test failed".into());

        let labels =
            label_rows_with_provider(&[row], &LabelerOptions::default(), Some(&FakeProvider))
                .await
                .unwrap();

        assert!(labels[0].labels.failure);
        assert!(labels[0].labels.verification_needed);
    }

    #[tokio::test]
    async fn llm_labeler_chunks_large_inputs() {
        let provider = ChunkingProvider::default();
        let rows = (0..3)
            .map(|idx| {
                WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id(format!("r{idx}"))
            })
            .collect::<Vec<_>>();
        let options = LabelerOptions {
            mode: LabelerMode::Llm,
            max_events_per_prompt: 1,
            ..LabelerOptions::default()
        };

        let labels = label_rows_with_provider(&rows, &options, Some(&provider))
            .await
            .unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].row_id, "r0");
        assert_eq!(labels[2].row_id, "r2");
        assert!(labels.iter().all(|label| label.labels.retry));
    }

    #[tokio::test]
    async fn llm_labeler_truncates_oversized_single_row() {
        let provider = ChunkingProvider::default();
        let mut row = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("huge");
        row.redacted_excerpt = Some("x".repeat(200_000));
        let options = LabelerOptions {
            mode: LabelerMode::Llm,
            max_prompt_chars: 4_000,
            ..LabelerOptions::default()
        };

        let labels = label_rows_with_provider(&[row], &options, Some(&provider))
            .await
            .unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(labels.len(), 1);
        assert!(provider.max_prompt_len.load(Ordering::SeqCst) <= options.max_prompt_chars);
        assert_eq!(provider.truncated_rows.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn llm_labeler_splits_batches_that_exceed_prompt_budget() {
        let provider = ChunkingProvider::default();
        let rows = (0..10)
            .map(|idx| {
                let mut row = WorldTraceRow::new("s1", WorldActionKind::ToolCall)
                    .with_row_id(format!("r{idx}"));
                row.redacted_excerpt = Some("x".repeat(10_000));
                row
            })
            .collect::<Vec<_>>();
        let options = LabelerOptions {
            mode: LabelerMode::Llm,
            max_prompt_chars: 4_000,
            max_events_per_prompt: 30,
            ..LabelerOptions::default()
        };

        let labels = label_rows_with_provider(&rows, &options, Some(&provider))
            .await
            .unwrap();

        assert!(provider.calls.load(Ordering::SeqCst) > 1);
        assert_eq!(labels.len(), rows.len());
        assert!(provider.max_prompt_len.load(Ordering::SeqCst) <= options.max_prompt_chars);
        assert_eq!(provider.truncated_rows.load(Ordering::SeqCst), rows.len());
    }

    #[test]
    fn label_response_accepts_fenced_json() {
        let labels = parse_label_response(
            r#"```json
{"rows":[{"row_id":"r1","labels":{"failure":true}}]}
```"#,
        )
        .unwrap();

        assert_eq!(labels[0].row_id, "r1");
        assert!(labels[0].labels.failure);
    }

    #[test]
    fn label_response_extracts_json_object_from_text() {
        let labels = parse_label_response(
            r#"Here are the labels:
{"rows":[{"row_id":"r1","labels":{"retry":true}}]}
Done."#,
        )
        .unwrap();

        assert_eq!(labels[0].row_id, "r1");
        assert!(labels[0].labels.retry);
    }
}
