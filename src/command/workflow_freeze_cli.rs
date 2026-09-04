//! Provider-bound CLI composition for task-set freeze commands.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{WorkflowLlmClientFactory, WorkflowLlmClientRequest};

use crate::cli_args::WorkflowAction;
use crate::command::workflow_freeze_candidate::{candidate_document, candidate_parse_error};

pub(super) async fn handle(
    action: &WorkflowAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    cwd: &Path,
) -> Result<bool> {
    match action {
        WorkflowAction::FreezeAcceptance {
            tasks,
            prd,
            candidate_stdin,
            staging_root,
            gate_envelope,
            call_id,
        } => {
            if staged_requested(
                *candidate_stdin,
                staging_root.as_deref(),
                gate_envelope.as_deref(),
                call_id.as_deref(),
            ) {
                let staged = require_staged_args(
                    *candidate_stdin,
                    staging_root.as_deref(),
                    gate_envelope.as_deref(),
                    call_id.as_deref(),
                )?;
                stage_acceptance(cwd, tasks, prd, config, env_vars, staged).await?;
            } else {
                freeze_acceptance(cwd, tasks, prd, config, env_vars).await?;
            }
            Ok(true)
        }
        WorkflowAction::FreezeSkeleton {
            tasks,
            prd,
            candidate_stdin,
            staging_root,
            gate_envelope,
            call_id,
        } => {
            if staged_requested(
                *candidate_stdin,
                staging_root.as_deref(),
                gate_envelope.as_deref(),
                call_id.as_deref(),
            ) {
                let staged = require_staged_args(
                    *candidate_stdin,
                    staging_root.as_deref(),
                    gate_envelope.as_deref(),
                    call_id.as_deref(),
                )?;
                stage_skeleton(cwd, tasks, prd, config, staged)?;
            } else {
                freeze_skeleton(cwd, tasks, prd, config)?;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[derive(Debug, Clone, Copy)]
struct StagedArgs<'a> {
    staging_root: &'a Path,
    gate_envelope: &'a Path,
    call_id: &'a str,
}

fn staged_requested(
    candidate_stdin: bool,
    staging_root: Option<&Path>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
) -> bool {
    candidate_stdin || staging_root.is_some() || gate_envelope.is_some() || call_id.is_some()
}

fn require_staged_args<'a>(
    candidate_stdin: bool,
    staging_root: Option<&'a Path>,
    gate_envelope: Option<&'a Path>,
    call_id: Option<&'a str>,
) -> Result<StagedArgs<'a>> {
    if !candidate_stdin {
        return Err(anyhow!(
            "trusted staged freeze requires --candidate-stdin with all staged fields"
        ));
    }
    let staging_root = staging_root
        .ok_or_else(|| anyhow!("trusted staged freeze requires --staging-root <DIR>"))?;
    let gate_envelope = gate_envelope
        .ok_or_else(|| anyhow!("trusted staged freeze requires --gate-envelope <PATH>"))?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("trusted staged freeze requires --call-id <ID>"))?;
    Ok(StagedArgs {
        staging_root,
        gate_envelope,
        call_id,
    })
}

async fn stage_acceptance(
    cwd: &Path,
    tasks: &Path,
    prd: &Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    staged: StagedArgs<'_>,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged acceptance preparation"
        ));
    }
    let candidate = read_bounded_stdin(archon_workflow::HostCommandRequest::MAX_STDIN_BYTES)?;
    let tasks_root = absolute(cwd, tasks);
    let prd_path = absolute(cwd, prd);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        );
    let client = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-decompose-freeze-acceptance".into(),
            session_id: staged.call_id.to_string(),
        })
        .await
        .context("building the staged acceptance judge client")?;
    if let Some(reason) =
        candidate_parse_error::<archon_workflow::task_set_contract::AcceptanceContract>(&candidate)
    {
        return refuse_candidate_artifact(
            cwd,
            staged,
            "freeze-acceptance",
            crate::command::workflow_gate::GateId::FreezeAcceptance,
            "acceptance",
            &reason,
        );
    }
    let prepared =
        match crate::command::workflow_task_set::prepare_acceptance_freeze_from_candidate(
            cwd,
            &tasks_root,
            &prd_path,
            config.workflow.gate_mode,
            candidate_document(&candidate).into_owned(),
            client,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(error) if crate::command::workflow_task_set::CandidateRejected::caused(&error) => {
                return refuse_candidate_artifact(
                    cwd,
                    staged,
                    "freeze-acceptance",
                    crate::command::workflow_gate::GateId::FreezeAcceptance,
                    "acceptance",
                    &format!("{error:#}"),
                );
            }
            Err(error) => {
                return report_operational_failure(cwd, staged, "freeze-acceptance", &error);
            }
        };
    let (evaluation, outputs) = prepared.into_staged_parts();
    write_staged_manifest(cwd, staged, "freeze-acceptance", evaluation, outputs)
}

fn stage_skeleton(
    cwd: &Path,
    tasks: &Path,
    prd: &Path,
    config: &ArchonConfig,
    staged: StagedArgs<'_>,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged skeleton preparation"
        ));
    }
    let candidate = read_bounded_stdin(archon_workflow::HostCommandRequest::MAX_STDIN_BYTES)?;
    let tasks_root = absolute(cwd, tasks);
    let prd_path = absolute(cwd, prd);
    if let Some(reason) =
        candidate_parse_error::<archon_workflow::task_skeleton::TaskSkeleton>(&candidate)
    {
        return refuse_candidate_artifact(
            cwd,
            staged,
            "freeze-skeleton",
            crate::command::workflow_gate::GateId::FreezeSkeleton,
            "skeleton",
            &reason,
        );
    }
    let prepared = match crate::command::workflow_task_set::prepare_skeleton_freeze_from_candidate(
        cwd,
        &tasks_root,
        &prd_path,
        config.workflow.gate_mode,
        candidate_document(&candidate).into_owned(),
    ) {
        Ok(prepared) => prepared,
        Err(error) if crate::command::workflow_task_set::CandidateRejected::caused(&error) => {
            return refuse_candidate_artifact(
                cwd,
                staged,
                "freeze-skeleton",
                crate::command::workflow_gate::GateId::FreezeSkeleton,
                "skeleton",
                &format!("{error:#}"),
            );
        }
        Err(error) => return report_operational_failure(cwd, staged, "freeze-skeleton", &error),
    };
    let (evaluation, outputs) = prepared.into_staged_parts();
    write_staged_manifest(cwd, staged, "freeze-skeleton", evaluation, outputs)
}

/// Refuse a candidate the host cannot even deserialize, as an authoritative
/// finding rather than a process failure.
///
/// The author owns the artifact; the host owns the judgement. A candidate that
/// is not the expected JSON document is an artifact problem, so it has to reach
/// the responsible author through the same findings channel every other
/// candidate defect uses — that is what lets the fixed retry loop correct it.
/// Exiting non-zero instead ended the run with no envelope, no receipt and no
/// way back, which is how a model that wraps correct JSON in prose or a code
/// fence killed a whole decomposition.
/// Report a staged command that failed operationally through its envelope.
///
/// The host owns the reason a phase stopped. Exiting non-zero throws that
/// reason away: the parent sees no envelope, the script reports the *absence*
/// of a receipt, and the operator is told "no committed publication receipt"
/// when the truth was a truncated judge response or an unreadable pin. The
/// envelope already carries `operational_error` and the script already stops on
/// it, so the honest failure travels the channel built for it.
fn report_operational_failure(
    cwd: &Path,
    staged: StagedArgs<'_>,
    command_id: &str,
    error: &anyhow::Error,
) -> Result<()> {
    write_staged_manifest(
        cwd,
        staged,
        command_id,
        crate::command::workflow_gate::GateEvaluation::new(
            "staged command failed operationally",
            Vec::new(),
        )
        .with_operational_error(format!("{error:#}")),
        Vec::new(),
    )
}

fn refuse_candidate_artifact(
    cwd: &Path,
    staged: StagedArgs<'_>,
    command_id: &str,
    gate_id: crate::command::workflow_gate::GateId,
    subject: &str,
    reason: &str,
) -> Result<()> {
    let finding = crate::command::workflow_gate::GateFinding::new(
        gate_id,
        format!(
            "candidate artifact was refused: {reason}. Return the artifact alone as raw JSON, with no prose, commentary or code fences, matching the required shape exactly."
        ),
        subject,
        None,
        archon_workflow::RemediationScope::CandidateArtifact,
    );
    write_staged_manifest(
        cwd,
        staged,
        command_id,
        crate::command::workflow_gate::GateEvaluation::new(
            "candidate refused before staging",
            vec![finding],
        ),
        Vec::new(),
    )
}

fn write_staged_manifest(
    cwd: &Path,
    staged: StagedArgs<'_>,
    command_id: &str,
    evaluation: crate::command::workflow_gate::GateEvaluation,
    outputs: Vec<(String, Vec<u8>)>,
) -> Result<()> {
    let outputs = outputs
        .into_iter()
        .map(
            |(relative_path, bytes)| crate::command::workflow_gate_envelope::StagedGateOutput {
                relative_path,
                bytes,
            },
        )
        .collect();
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staged.staging_root,
        staged.gate_envelope,
        staged.call_id,
        command_id,
        evaluation,
        outputs,
    )?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

fn read_bounded_stdin(limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .context("reading trusted candidate stdin")?;
    if bytes.len() > limit {
        return Err(anyhow!("candidate stdin exceeds {limit} bytes"));
    }
    Ok(bytes)
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

#[cfg(test)]
#[path = "workflow_freeze_cli_tests.rs"]
mod tests;
