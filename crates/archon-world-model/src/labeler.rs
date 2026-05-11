//! Provider-neutral semantic labeler for world-model rows.

use anyhow::{Result, bail};
use archon_llm::provider::{LlmProvider, LlmRequest};
use serde::{Deserialize, Serialize};

use crate::labels::DeterministicLabelBuilder;
use crate::schema::{WorldLabelSet, WorldTraceRow};

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
            max_events_per_prompt: 120,
            max_prompt_chars: 24_000,
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
            llm_label_rows(
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
                merge_llm_labels(&mut labels, llm_label_rows(rows, options, provider).await?);
            }
            Ok(labels)
        }
    }
}

async fn llm_label_rows(
    rows: &[WorldTraceRow],
    options: &LabelerOptions,
    provider: &dyn LlmProvider,
) -> Result<Vec<RowLabelUpdate>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let prompt = label_prompt(rows, options)?;
    let response = provider
        .complete(LlmRequest {
            model: options.model.clone(),
            max_tokens: 2048,
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

fn label_prompt(rows: &[WorldTraceRow], options: &LabelerOptions) -> Result<String> {
    let mut events = Vec::new();
    for row in rows.iter().take(options.max_events_per_prompt) {
        events.push(serde_json::json!({
            "row_id": row.row_id,
            "source": row.source,
            "action_kind": row.action_kind,
            "provider": row.provider,
            "model": row.model,
            "agent": row.agent,
            "excerpt": row.redacted_excerpt,
            "labels": row.labels,
            "scalar": row.scalar_features,
        }));
    }
    let prompt = serde_json::json!({
        "task": "Return labels for each row. Preserve row_id. Use booleans for failure, retry, provider_incident, verification_needed, user_correction, plan_drift, high_cost, slow_run and optional success.",
        "schema": {"rows": [{"row_id": "string", "labels": {}}]},
        "rows": events,
    })
    .to_string();
    if prompt.len() > options.max_prompt_chars {
        bail!("world-model label prompt exceeds max_prompt_chars");
    }
    Ok(prompt)
}

fn parse_label_response(text: &str) -> Result<Vec<RowLabelUpdate>> {
    let response: LlmLabelResponse = serde_json::from_str(text.trim())
        .map_err(|error| anyhow::anyhow!("invalid world-model label JSON: {error}"))?;
    Ok(response.rows)
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
}
