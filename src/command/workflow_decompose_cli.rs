//! First-class CLI composition for fixed decomposition launch and resume.

use std::path::Path;

use anyhow::{Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;

use crate::cli_args::WorkflowAction;

pub(super) async fn handle(
    action: &WorkflowAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    cwd: &Path,
) -> Result<bool> {
    match action {
        WorkflowAction::ReclaimTaskRoot { run_id, yes } => {
            println!("{}", crate::command::workflow_task_root_reclaim::reclaim(cwd, run_id, *yes)?);
            Ok(true)
        }
        WorkflowAction::VerifyFrozenChain {
            stage,
            tasks,
            prd,
            gate_envelope,
            call_id,
        } => {
            crate::command::workflow_decompose_frozen_chain::handle_staged_verify(
                cwd,
                stage,
                tasks,
                prd,
                gate_envelope.as_deref(),
                call_id.as_deref(),
                config,
            )?;
            Ok(true)
        }
        WorkflowAction::DecompositionIdentity => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &crate::command::workflow_decompose_identity::fixed_decomposition_identity()?
                )?
            );
            Ok(true)
        }
        WorkflowAction::Decompose {
            prd,
            tasks,
            repository,
            yes,
        } => {
            let factory = crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
                config, env_vars,
            );
            let output = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
                cwd,
                prd,
                tasks,
                repository.as_deref(),
                *yes,
                config,
                env_vars,
                &factory,
            )
            .await?;
            println!("{output}");
            Ok(true)
        }
        WorkflowAction::Resume { live, yes, run_id }
            if crate::command::workflow_decompose::is_fixed_decomposition_run(cwd, run_id)? =>
        {
            if !(*live && *yes) {
                return Err(anyhow!(
                    "fixed workflow resume requires --live --yes before the run can execute"
                ));
            }
            let factory = crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
                config, env_vars,
            );
            let output =
                crate::command::workflow_decompose::resume_fixed_decomposition_with_factory(
                    cwd, run_id, *yes, config, env_vars, &factory,
                )
                .await?;
            println!("{output}");
            Ok(true)
        }
        _ => Ok(false),
    }
}
