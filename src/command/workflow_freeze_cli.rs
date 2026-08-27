//! Provider-bound CLI composition for task-set freeze commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{WorkflowLlmClientFactory, WorkflowLlmClientRequest};

use crate::cli_args::WorkflowAction;

pub(super) async fn handle(
    action: &WorkflowAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    cwd: &Path,
) -> Result<bool> {
    match action {
        WorkflowAction::FreezeAcceptance { tasks, prd } => {
            freeze_acceptance(cwd, tasks, prd, config, env_vars).await?;
            Ok(true)
        }
        WorkflowAction::FreezeSkeleton { tasks, prd } => {
            freeze_skeleton(cwd, tasks, prd, config)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn freeze_acceptance(
    cwd: &Path,
    tasks: &Path,
    prd: &Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        print!("{}", crate::command::workflow_gate::OFF_MESSAGE);
        return Ok(());
    }
    let tasks_root = absolute(cwd, tasks);
    let prd_path = absolute(cwd, prd);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::new(config, env_vars);
    let client = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-freeze-acceptance".into(),
            session_id: format!("acceptance-freeze-{}", uuid::Uuid::new_v4()),
        })
        .await
        .context("building the batched acceptance judge client")?;
    let prepared = crate::command::workflow_task_set::prepare_acceptance_freeze(
        cwd,
        &tasks_root,
        &prd_path,
        config.workflow.gate_mode,
        client,
    )
    .await?;
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        cwd,
        config.workflow.gate_mode,
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )?;
    for diagnostic in disposition.diagnostics() {
        eprintln!("{diagnostic}");
    }
    disposition.require_allowed()?;
    let permit = disposition
        .take_publication_permit()
        .ok_or_else(|| anyhow!("acceptance freeze received no publication permit"))?;
    let result = crate::command::workflow_task_set::publish_acceptance_freeze(prepared, permit)?;
    println!(
        "acceptance contract frozen: digest={} event={}",
        result.acceptance_digest, result.freeze_event_id
    );
    Ok(())
}

fn freeze_skeleton(cwd: &Path, tasks: &Path, prd: &Path, config: &ArchonConfig) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        print!("{}", crate::command::workflow_gate::OFF_MESSAGE);
        return Ok(());
    }
    let tasks_root = absolute(cwd, tasks);
    let prd_path = absolute(cwd, prd);
    let prepared = crate::command::workflow_task_set::prepare_skeleton_freeze(
        cwd,
        &tasks_root,
        &prd_path,
        config.workflow.gate_mode,
    )?;
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        cwd,
        config.workflow.gate_mode,
        crate::command::workflow_gate::GateId::FreezeSkeleton,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )?;
    for diagnostic in disposition.diagnostics() {
        eprintln!("{diagnostic}");
    }
    disposition.require_allowed()?;
    let permit = disposition
        .take_publication_permit()
        .ok_or_else(|| anyhow!("skeleton freeze received no publication permit"))?;
    let result = crate::command::workflow_task_set::publish_skeleton_freeze(prepared, permit)?;
    println!(
        "task skeleton frozen: digest={} acceptance_digest={}",
        result.skeleton_digest, result.acceptance_digest
    );
    Ok(())
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
