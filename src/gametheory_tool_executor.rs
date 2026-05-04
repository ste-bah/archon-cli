//! Concrete executor for the agent-callable game-theory tools.

use std::sync::Arc;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::gametheory;
use archon_pipeline::runner::LlmClient;
use archon_tools::gametheory::{
    GameTheoryCallSpecialistRequest, GameTheoryClassifyRequest, GameTheoryExecutor,
    GameTheoryInspectRequest, GameTheoryListAgentsRequest, GameTheoryReplayRequest,
    GameTheoryRunRequest, GameTheorySpecimensRequest, GameTheoryStatusRequest,
};
use async_trait::async_trait;

use crate::command::{gametheory as cli_gametheory, gametheory_inspect};

#[derive(Clone)]
struct PipelineGameTheoryExecutor {
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
}

pub(crate) fn install(config: ArchonConfig, env_vars: ArchonEnvVars) {
    archon_tools::gametheory::install_gametheory_executor(Arc::new(PipelineGameTheoryExecutor {
        config,
        env_vars,
    }));
}

#[async_trait]
impl GameTheoryExecutor for PipelineGameTheoryExecutor {
    async fn run(&self, request: GameTheoryRunRequest) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        let llm = self.build_llm();
        let llm_ref = llm.as_ref().map(|client| client as &dyn LlmClient);
        let result = gametheory::run_full_pipeline_with_options(
            &db,
            &request.situation,
            None,
            llm_ref,
            cli_gametheory::open_memory_context(false)?,
            gametheory::GameTheoryRunOptions {
                budget_usd: request.budget_usd.unwrap_or(20.0),
                max_concurrent: request.max_concurrent.unwrap_or(4),
                style_profile_id: Some(request.style.unwrap_or_else(|| "executive".into())),
            },
        )
        .await?;
        let persisted_status = gametheory_inspect::render_status(&db, Some(&result.run_id))?;
        Ok(format!(
            "GameTheoryRun persisted run_id={} status={} specialists={} cost=${:.6}\n\n{}",
            result.run_id,
            result.status,
            result.specialist_count,
            result.total_cost_usd,
            persisted_status
        ))
    }

    async fn status(&self, request: GameTheoryStatusRequest) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        gametheory_inspect::render_status(&db, request.run_id.as_deref())
    }

    async fn list_agents(&self, request: GameTheoryListAgentsRequest) -> Result<String> {
        gametheory_inspect::render_list_agents(request.tier)
    }

    async fn specimens(&self, request: GameTheorySpecimensRequest) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        let load = gametheory::specimens::ensure_specimen_library_loaded(&db, request.ingest)?;
        let rows = gametheory::specimens::list_specimens(&db, request.filter.as_deref())?;
        let mut out = format!(
            "GameTheorySpecimens source=gt_specimen_library rows={} inserted={}\n",
            rows.len(),
            load.inserted
        );
        for row in rows {
            out.push_str(&format!(
                "- {} cooperation={} payoff_sum={} timing={} horizon={}\n",
                row.situation_type, row.cooperation, row.payoff_sum, row.timing, row.horizon
            ));
        }
        Ok(out)
    }

    async fn inspect(&self, request: GameTheoryInspectRequest) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        gametheory_inspect::render_inspect_artifact(&db, &request.artifact_id)
    }

    async fn replay(&self, request: GameTheoryReplayRequest) -> Result<String> {
        if request.reclassify {
            return self.reclassify_run(&request.run_id).await;
        }
        if let Some(agent_key) = request.rerun_specialist {
            return self.rerun_specialist(&request.run_id, &agent_key).await;
        }
        let db = cli_gametheory::open_db()?;
        let routing =
            gametheory::replay_routing_from_stored_fingerprint(&db, &request.run_id, None)?;
        Ok(format!(
            "GameTheoryReplay persisted routing for {} enabled={} skipped={}",
            request.run_id,
            routing.enabled_specialists.len(),
            routing.skipped_specialists.len()
        ))
    }

    async fn classify(&self, request: GameTheoryClassifyRequest) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        let llm = self.build_llm();
        let llm_ref = llm.as_ref().map(|client| client as &dyn LlmClient);
        let fingerprint = gametheory::classify(&db, &request.situation, llm_ref).await?;
        let artifact_id = format!("fingerprint:{}", fingerprint.run_id);
        let persisted = gametheory_inspect::render_inspect_artifact(&db, &artifact_id)?;
        Ok(format!(
            "GameTheoryClassify persisted run_id={} primary_family={}\n\n{}",
            fingerprint.run_id, fingerprint.primary_family, persisted
        ))
    }

    async fn call_specialist(&self, request: GameTheoryCallSpecialistRequest) -> Result<String> {
        self.rerun_specialist(&request.run_id, &request.agent_key)
            .await
    }
}

impl PipelineGameTheoryExecutor {
    fn build_llm(&self) -> Option<archon_pipeline::llm_adapter::AnthropicLlmAdapter> {
        cli_gametheory::build_llm_client(&self.config, &self.env_vars)
    }

    async fn reclassify_run(&self, run_id: &str) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        let Some(situation) = gametheory_inspect::load_run_situation(&db, run_id)? else {
            anyhow::bail!("run not found: {run_id}");
        };
        self.run(GameTheoryRunRequest {
            situation,
            budget_usd: Some(20.0),
            max_concurrent: Some(4),
            style: Some("executive".into()),
        })
        .await
    }

    async fn rerun_specialist(&self, run_id: &str, agent_key: &str) -> Result<String> {
        let db = cli_gametheory::open_db()?;
        let llm = self.build_llm();
        let llm_ref = llm.as_ref().map(|client| client as &dyn LlmClient);
        let result = gametheory::replay_single_specialist(
            &db,
            run_id,
            agent_key,
            llm_ref,
            cli_gametheory::open_memory_context(false)?,
            gametheory::GameTheoryRunOptions::default(),
        )
        .await?;
        let artifact_id = format!("specialist:{run_id}:{}", result.agent_key);
        let persisted = gametheory_inspect::render_inspect_artifact(&db, &artifact_id)?;
        Ok(format!(
            "GameTheoryCallSpecialist persisted run_id={} agent={} status={} cost=${:.6}\n\n{}",
            result.run_id, result.agent_key, result.status, result.cost_usd, persisted
        ))
    }
}
