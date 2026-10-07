//! Provider-bound CLI composition for task-set freeze commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{WorkflowLlmClientFactory, WorkflowLlmClientRequest};

use crate::cli_args::WorkflowAction;
use crate::cli_args::{WorkflowFreezeAcceptanceArgs, WorkflowFreezeSkeletonArgs};
use crate::command::workflow_freeze_candidate::{candidate_document, candidate_refusal};

#[path = "workflow_freeze_acceptance_cli.rs"]
mod acceptance;

pub(super) async fn handle(
    action: &WorkflowAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    cwd: &Path,
) -> Result<bool> {
    match action {
        WorkflowAction::FreezeAcceptance(WorkflowFreezeAcceptanceArgs {
            tasks,
            prd,
            reauthor,
            candidate_stdin,
            staging_root,
            gate_envelope,
            call_id,
        }) => {
            let staged = staged_requested(
                *candidate_stdin,
                staging_root.as_deref(),
                gate_envelope.as_deref(),
                call_id.as_deref(),
            );
            if !reauthor.is_empty() {
                if staged {
                    return Err(anyhow!(
                        "--reauthor repairs a frozen contract in place and cannot be combined with staged freeze flags"
                    ));
                }
                acceptance::reauthor_acceptance(cwd, tasks, prd, reauthor, config, env_vars)
                    .await?;
            } else if staged {
                let staged = require_staged_args(
                    *candidate_stdin,
                    staging_root.as_deref(),
                    gate_envelope.as_deref(),
                    call_id.as_deref(),
                )?;
                stage_acceptance(cwd, tasks, prd, config, env_vars, staged).await?;
            } else {
                acceptance::freeze_acceptance(cwd, tasks, prd, config, env_vars).await?;
            }
            Ok(true)
        }
        WorkflowAction::FreezeSkeleton(WorkflowFreezeSkeletonArgs {
            tasks,
            prd,
            candidate_stdin,
            staging_root,
            gate_envelope,
            call_id,
        }) => {
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
    let resume = staged_freeze_resume(&mut std::io::stderr());
    let candidate = read_bounded_stdin(archon_workflow::HostCommandRequest::MAX_STDIN_BYTES)?;
    let tasks_root = absolute(cwd, tasks);
    let prd_path = absolute(cwd, prd);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        )
        .without_project_tools();
    let client = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-decompose-freeze-acceptance".into(),
            session_id: staged.call_id.to_string(),
            read_roots: Vec::new(),
        })
        .await
        .context("building the staged acceptance judge client")?;
    let gate = (
        "freeze-acceptance",
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        "acceptance",
    );
    // Every entry's fields first: assembly reads the entries vector whole.
    let shape = &defects::ENTRY_SHAPE;
    if let Some(refused) = defects::refuse_element_shapes(cwd, staged, gate, &candidate, shape) {
        return refused;
    }
    let candidate =
        match crate::command::workflow_freeze_candidate::acceptance_candidate_for_validation(
            &candidate,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                return defects::refuse_candidate_error(
                    cwd,
                    staged,
                    "freeze-acceptance",
                    crate::command::workflow_gate::GateId::FreezeAcceptance,
                    "acceptance",
                    &error,
                );
            }
        };
    if let Some((code, reason)) =
        candidate_refusal::<archon_workflow::task_set_contract::AcceptanceContract>(&candidate)
    {
        return refuse_candidate_artifact(
            cwd,
            staged,
            "freeze-acceptance",
            crate::command::workflow_gate::GateId::FreezeAcceptance,
            "acceptance",
            code,
            &reason,
        );
    }
    let prepared = match crate::command::workflow_task_set::prepare_acceptance_freeze_resumable(
        cwd,
        &tasks_root,
        &prd_path,
        config.workflow.gate_mode,
        candidate_document(&candidate).into_owned(),
        client,
        &resume,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) if crate::command::workflow_task_set::CandidateRejected::caused(&error) => {
            return defects::refuse_candidate_error(
                cwd,
                staged,
                "freeze-acceptance",
                crate::command::workflow_gate::GateId::FreezeAcceptance,
                "acceptance",
                &error,
            );
        }
        Err(error) => {
            if let Some(incomplete) =
                crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
            {
                exit_incomplete_resumable(incomplete);
            }
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
    let gate = (
        "freeze-skeleton",
        crate::command::workflow_gate::GateId::FreezeSkeleton,
        "skeleton",
    );
    if let Some(refused) =
        defects::refuse_element_shapes(cwd, staged, gate, &candidate, &defects::TASK_SHAPE)
    {
        return refused;
    }
    if let Some((code, reason)) =
        candidate_refusal::<archon_workflow::task_skeleton::TaskSkeleton>(&candidate)
    {
        return refuse_candidate_artifact(
            cwd,
            staged,
            "freeze-skeleton",
            crate::command::workflow_gate::GateId::FreezeSkeleton,
            "skeleton",
            code,
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
            return defects::refuse_candidate_error(
                cwd,
                staged,
                "freeze-skeleton",
                crate::command::workflow_gate::GateId::FreezeSkeleton,
                "skeleton",
                &error,
            );
        }
        Err(error) => return report_operational_failure(cwd, staged, "freeze-skeleton", &error),
    };
    let (evaluation, outputs) = prepared.into_staged_parts();
    write_staged_manifest(cwd, staged, "freeze-skeleton", evaluation, outputs)
}

/// The staged freeze's budget and progress (Issue 255): the host's wall
/// clock for `freeze-acceptance`, counted from here, and a progress
/// baseline on `stderr` before anything slow (stdin, the judge client, the
/// probe) runs. Without it an attempt the host kills before its first saved
/// verdict reports no progress, and the executor reads the next attempt's
/// saved work as no evidence and pauses instead of retrying. The baseline
/// is 0, not the count of verdicts a retry will reuse: their keys need the
/// probe site and tree, which exist only later.
fn staged_freeze_resume(
    stderr: &mut dyn std::io::Write,
) -> crate::command::workflow_freeze_budget::FreezeResume {
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let _ = writeln!(stderr, "{}", resume.progress.line());
    resume
}

/// End an incomplete, resumable freeze (Issue 255) by the host's
/// operational contract (`workflow_host_command_operational`): the reason
/// and the progress line on stderr, then `EXIT_INCOMPLETE_RESUMABLE`. The
/// executor retries the call while progress grows and otherwise pauses the
/// run; nothing staged is published, so no envelope is written.
fn exit_incomplete_resumable(
    incomplete: &crate::command::workflow_freeze_budget::FreezeIncomplete,
) -> ! {
    eprintln!("{}", incomplete.report());
    std::process::exit(crate::command::workflow_host_command_operational::EXIT_INCOMPLETE_RESUMABLE)
}

#[path = "workflow_freeze_defects.rs"]
mod defects;
pub(crate) use defects::{install_entry_validator, skeleton_document};
#[path = "workflow_freeze_staged_output.rs"]
mod staged_output;
use staged_output::{
    read_bounded_stdin, refuse_candidate_artifact, report_operational_failure,
    write_staged_manifest,
};

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

/// `path` resolved against `cwd`, refused at the command line when it is
/// empty or names nothing. `flag` names the option in the error. An empty
/// `--prd ""` used to resolve to `cwd` itself, and the run failed far from
/// the input that caused it.
fn required_path(cwd: &Path, path: &Path, flag: &str) -> Result<PathBuf> {
    if path.as_os_str().to_string_lossy().trim().is_empty() {
        return Err(anyhow!("{flag} is empty; it must name an existing path"));
    }
    let resolved = absolute(cwd, path);
    if !resolved.exists() {
        return Err(anyhow!(
            "{flag} names {}, which does not exist",
            resolved.display()
        ));
    }
    Ok(resolved)
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

#[cfg(test)]
#[path = "workflow_freeze_cli_baseline_tests.rs"]
pub(crate) mod baseline_tests;

// Issue 360: a resume checks every carried acceptance entry with this
// build's freeze entry validator before it carries it.
pub(crate) use defects::{ENTRY_SHAPE, element_shape_defects};
