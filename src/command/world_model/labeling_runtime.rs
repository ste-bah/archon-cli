use std::sync::Arc;

use anyhow::Result;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::provider::LlmProvider;
use archon_world_model::ingest::{IngestSummary, IngestWarning};
use archon_world_model::labeler::{LabelerMode, LabelerOptions, label_rows_with_provider};

pub(super) struct WorldModelLabelingRuntime {
    options: LabelerOptions,
    provider: Option<Arc<dyn LlmProvider>>,
    disabled_reason: Option<String>,
}

impl WorldModelLabelingRuntime {
    pub(super) async fn from_config(
        config: &archon_core::config::ArchonConfig,
        env_vars: &ArchonEnvVars,
    ) -> Result<Self> {
        let labeler = &config.learning.world_model.labeler;
        let configured_mode = parse_labeler_mode(&labeler.analyzer);
        let mut options = LabelerOptions {
            mode: configured_mode,
            model: config.api.default_model.clone(),
            max_events_per_prompt: labeler.max_events_per_prompt,
            max_prompt_chars: labeler.max_prompt_chars,
        };
        if !labeler.llm_enabled || configured_mode == LabelerMode::Heuristic {
            options.mode = LabelerMode::Heuristic;
            return Ok(Self {
                options,
                provider: None,
                disabled_reason: (!labeler.llm_enabled)
                    .then_some("world-model LLM labeler disabled by config".into()),
            });
        }

        let policy =
            archon_policy::load_effective_policy(&std::env::current_dir()?).unwrap_or_default();
        let decision = policy.world_model_llm_labeler_decision();
        if !decision.allowed {
            options.mode = LabelerMode::Heuristic;
            return Ok(Self {
                options,
                provider: None,
                disabled_reason: Some(decision.reason),
            });
        }

        match crate::runtime::llm::build_configured_llm_provider(
            config,
            env_vars,
            "world-model-labeler",
        )
        .await
        {
            Ok(provider) => Ok(Self {
                options,
                provider: Some(provider),
                disabled_reason: None,
            }),
            Err(error) => {
                options.mode = LabelerMode::Heuristic;
                Ok(Self {
                    options,
                    provider: None,
                    disabled_reason: Some(format!("world-model LLM labeler unavailable: {error}")),
                })
            }
        }
    }

    pub(super) async fn apply(&self, summary: &mut IngestSummary) -> Result<()> {
        if summary.rows.is_empty() {
            return Ok(());
        }
        if let Some(reason) = &self.disabled_reason {
            summary.warnings.push(IngestWarning {
                line: None,
                message: reason.clone(),
            });
        }
        let updates =
            label_rows_with_provider(&summary.rows, &self.options, self.provider.as_deref())
                .await?;
        for update in updates {
            if let Some(row) = summary
                .rows
                .iter_mut()
                .find(|row| row.row_id == update.row_id)
            {
                row.labels = update.labels;
            }
        }
        Ok(())
    }
}

fn parse_labeler_mode(value: &str) -> LabelerMode {
    match value {
        "llm" => LabelerMode::Llm,
        "heuristic" => LabelerMode::Heuristic,
        _ => LabelerMode::Hybrid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn config_disabled_labeler_falls_back_to_heuristics() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.labeler.llm_enabled = false;
        let env_vars = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
        let runtime = WorldModelLabelingRuntime::from_config(&config, &env_vars)
            .await
            .unwrap();

        let mut summary = IngestSummary {
            rows: vec![
                archon_world_model::schema::WorldTraceRow::new(
                    "s1",
                    archon_world_model::schema::WorldActionKind::ToolCall,
                )
                .with_row_id("r1"),
            ],
            ..IngestSummary::default()
        };
        summary.rows[0].redacted_excerpt = Some("test failed".into());
        runtime.apply(&mut summary).await.unwrap();

        assert!(summary.rows[0].labels.failure);
        assert_eq!(summary.warning_count(), 1);
    }
}
