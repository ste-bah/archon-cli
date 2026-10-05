//! Provider-bound CLI for the acceptance freeze and its per-check repair.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{WorkflowLlmClient, WorkflowLlmClientFactory, WorkflowLlmClientRequest};

use super::required_path;
use crate::command::workflow_task_set::executability::HostProbe;
use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate};
use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};

async fn client(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    cwd: &Path,
    origin: &str,
    scope: &AuthorScope,
) -> Result<std::sync::Arc<dyn WorkflowLlmClient>> {
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::new(config, env_vars)
            .without_project_tools();
    factory
        .build_client(WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: origin.into(),
            session_id: format!("{origin}-{}", uuid::Uuid::new_v4()),
            // The author reads the repository the task set was decomposed
            // against, which may not be the project directory.
            read_roots: vec![scope.repository_root.clone()],
        })
        .await
        .context("building the acceptance author and judge client")
}

pub(super) async fn freeze_acceptance(
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
    let tasks_root = required_path(cwd, tasks, "--tasks")?;
    let prd_path = required_path(cwd, prd, "--prd")?;
    let scope = AuthorScope::for_task_set(cwd, &tasks_root, &prd_path);
    let client = client(config, env_vars, cwd, "workflow-freeze-acceptance", &scope).await?;
    let prepared = crate::command::workflow_task_set::prepare_acceptance_freeze_reauthoring(
        cwd,
        &tasks_root,
        &prd_path,
        config.workflow.gate_mode,
        client,
        &scope,
    )
    .await
    .map_err(exit_if_incomplete)?;
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

pub(super) async fn reauthor_acceptance(
    cwd: &Path,
    tasks: &Path,
    prd: &Path,
    ids: &[String],
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    // A repair republishes a gated freeze; with gates off there is no gate to
    // run it through, and exiting 0 would look like a repair that happened.
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off: --reauthor republishes a gated freeze and runs each gate in the mode its stage was frozen in; enable gates (observe or enforce) and re-run"
        ));
    }
    let tasks_root = required_path(cwd, tasks, "--tasks")?;
    let prd_path = required_path(cwd, prd, "--prd")?;
    let ids: BTreeSet<String> = ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    let scope = AuthorScope::for_task_set(cwd, &tasks_root, &prd_path);
    let client = client(
        config,
        env_vars,
        cwd,
        "workflow-freeze-acceptance-reauthor",
        &scope,
    )
    .await?;
    // Each re-authored check is run once in the hermetic scratch site before
    // it may be published.
    let probe = HostProbe::for_task_set(cwd, &tasks_root);
    let result = reauthor_and_republish(
        client.as_ref(),
        ReauthorRequest {
            project_root: cwd,
            tasks_root: &tasks_root,
            prd_path: &prd_path,
            ids: &ids,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &BTreeMap::new(),
            },
            trigger: "workflow freeze-acceptance --reauthor",
        },
        &scope,
    )
    .await
    .map_err(|error| {
        exit_if_incomplete(crate::command::workflow_task_set::unproven_incomplete(
            error,
        ))
    })?;
    for diagnostic in &result.diagnostics {
        eprintln!("{diagnostic}");
    }
    println!(
        "acceptance checks re-authored: {} digest={} event={} skeleton_digest={}",
        ids.iter().cloned().collect::<Vec<_>>().join(","),
        result.acceptance_digest,
        result.freeze_event_id,
        result.skeleton_digest.as_deref().unwrap_or("none")
    );
    Ok(())
}

/// An incomplete, resumable freeze ends by the operational contract (Issue
/// 288), as the staged freeze does: its reason and progress on stderr, then
/// `EXIT_INCOMPLETE_RESUMABLE`. Any other error is returned.
fn exit_if_incomplete(error: anyhow::Error) -> anyhow::Error {
    if let Some(incomplete) =
        crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
    {
        super::exit_incomplete_resumable(incomplete);
    }
    error
}
