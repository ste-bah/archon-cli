//! Hidden trusted-child composition for fixed-decomposition lint capabilities.

use std::path::Path;

use anyhow::{Result, anyhow};
use archon_workflow::WorkflowLlmClientFactory;

#[path = "workflow_staged_cli_task_set_error.rs"]
mod task_set_error;

#[cfg(test)]
#[path = "workflow_staged_cli_tests.rs"]
mod tests;

#[path = "workflow_staged_candidate.rs"]
pub(super) mod candidate;
#[path = "workflow_staged_gate_resume.rs"]
mod gate_resume;

/// The decomposition's set gate: the point at which a task set with bodies is
/// accepted, so the obligation fidelity audit runs here unconditionally.
/// The critic client comes from the configured provider only — the child runs
/// under the freeze provider environment the parent resolved, never an
/// ambient one — and a client that cannot be built reaches the envelope as an
/// operational error rather than a pass: a provider outage must stop the
/// freeze, not certify it. Waivers are the ones an operator recorded in the
/// task set's pin; the catalog argv is fixed, so there is no flag to pass here.
///
/// The critic calls run under the command's own catalog wall clock less the
/// freeze safety margin (Issue 259); see `topology_lint::fidelity_resume`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_staged_task_set_lint(
    cwd: &Path,
    task_file: Option<&Path>,
    tasks: Option<&Path>,
    spec_file: Option<&Path>,
    graph: Option<&str>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<()> {
    let resume = staged_set_gate_resume(&mut std::io::stderr());
    let mode = config.workflow.gate_mode;
    if mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged task-set lint"
        ));
    }
    if task_file.is_some() || spec_file.is_some() || graph.is_some() {
        return Err(anyhow!(
            "trusted staged task-set lint accepts only --tasks <DIR>"
        ));
    }
    let tasks =
        tasks.ok_or_else(|| anyhow!("trusted staged task-set lint requires --tasks <DIR>"))?;
    let gate_envelope = gate_envelope
        .ok_or_else(|| anyhow!("trusted staged task-set lint requires --gate-envelope <PATH>"))?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("trusted staged task-set lint requires --call-id <ID>"))?;
    let source = crate::command::topology_lint::LintSource::Tasks(tasks.to_path_buf());
    let tasks_root = if tasks.is_absolute() {
        tasks.to_path_buf()
    } else {
        cwd.join(tasks)
    };
    let waivers = crate::command::topology_lint::recorded_waivers(cwd, &tasks_root);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        )
        .without_project_tools();
    let client = factory
        .build_client(archon_workflow::WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-decompose-task-set-lint".into(),
            session_id: call_id.to_string(),
            read_roots: Vec::new(),
        })
        .await
        .map_err(anyhow::Error::new);
    let evaluation = match crate::command::topology_lint::evaluate_lint_with_fidelity_resumable(
        cwd, &source, mode, client, &waivers, &resume,
    )
    .await
    {
        Ok(evaluation) => evaluation,
        Err(error) => task_set_error::body_input_finding(&error)
            .unwrap_or_else(|| task_set_error::gate_error_evaluation(&error)),
    };
    let staging_root = gate_envelope
        .parent()
        .ok_or_else(|| anyhow!("staged task-set envelope has no parent"))?;
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        "task-set-lint",
        evaluation,
        Vec::new(),
    )?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

pub(super) use gate_resume::staged_set_gate_resume;

/// The decomposition's body gate (`land-task-body`).
///
/// The mechanical body checks run first; when they pass, the candidate is
/// judged for obligation fidelity against the obligations it claims
/// (Issue-44). The set gate keeps the same audit, but there a false verdict
/// stops the run: it has no author to send the finding back to. Here the
/// body author still has attempts, so each false verdict lands as a `Body`
/// finding it is asked to repair. The critic client, waivers, and the
/// operational-not-pass rule are exactly the set gate's.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_staged_task_file_lint(
    cwd: &Path,
    task_file: Option<&Path>,
    tasks: Option<&Path>,
    spec_file: Option<&Path>,
    graph: Option<&str>,
    candidate_stdin: bool,
    task_context_file: Option<&Path>,
    staging_root: Option<&Path>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<()> {
    let mode = config.workflow.gate_mode;
    if mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged task-file lint"
        ));
    }
    if !candidate_stdin {
        return Err(anyhow!(
            "trusted staged task-file lint requires --candidate-stdin"
        ));
    }
    if tasks.is_some() || spec_file.is_some() || graph.is_some() {
        return Err(anyhow!(
            "trusted staged task-file lint accepts only --task-file <PATH>"
        ));
    }
    let task_file = task_file
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --task-file <PATH>"))?;
    let staging_root = staging_root
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --staging-root <DIR>"))?;
    let gate_envelope = gate_envelope
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --gate-envelope <PATH>"))?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --call-id <ID>"))?;
    // Issue 386: stdin, or the run spill a bounded view on it names.
    let candidate = crate::command::workflow_host_command_stdin_spill::read_candidate(
        std::io::stdin(),
        archon_workflow::HostCommandRequest::MAX_STDIN_BYTES,
        staging_root,
    )?;
    let staged_path = if task_file.is_absolute() {
        task_file.to_path_buf()
    } else {
        cwd.join(task_file)
    };
    let path = task_context_file.map_or_else(
        || staged_path.clone(),
        |path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            }
        },
    );
    // Issue-61/367: the task file is the answer when it opens with its
    // frontmatter, or the inside of one outer fence; any other answer has
    // text before the task file and goes back to the author as one finding,
    // before any lint reads it and before anything is staged.
    let normalized = match candidate::normalize_task_candidate(candidate, &path) {
        Ok(candidate) => candidate,
        Err(reason) => {
            let manifest = candidate::stage_shape_refusal(
                staging_root,
                gate_envelope,
                call_id,
                &path,
                reason,
            )?;
            println!("{}", serde_json::to_string(&manifest)?);
            return Ok(());
        }
    };
    let candidate::TaskCandidate {
        bytes: candidate,
        packaging,
    } = normalized;
    let file_name = staged_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("candidate TASK path has no UTF-8 file name"))?
        .to_string();
    let mut evaluation = match crate::command::topology_lint::evaluate_task_file_candidate(
        cwd, &path, &candidate, mode,
    ) {
        Ok(evaluation) => evaluation,
        Err(error) => {
            let fallback_subject = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("task-file");
            let Some(evaluation) = crate::command::workflow_gate::task_body_input_finding(
                &error,
                crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                fallback_subject,
            ) else {
                return Err(error);
            };
            let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
                staging_root,
                gate_envelope,
                call_id,
                "land-task-body",
                evaluation,
                Vec::new(),
            )?;
            println!("{}", serde_json::to_string(&manifest)?);
            return Ok(());
        }
    };
    // Whatever the host removed is recorded, a wrapper pair included.
    if let Some(packaging) = &packaging {
        evaluation.report.push_str(&packaging.report());
    }
    // Inherited-predecessor notes ride on every body of a set whose earlier
    // freezes carried residuals, so they must not suppress the audit: only a
    // finding the author can act on here means the body is going back anyway.
    let actionable_findings = evaluation.findings.iter().any(|finding| {
        finding.remediation_scope != archon_workflow::RemediationScope::InheritedPredecessor
    });
    let evaluation = if !actionable_findings && evaluation.operational_error().is_none() {
        audit_candidate_fidelity(
            cwd, &path, &candidate, call_id, config, env_vars, evaluation,
        )
        .await
    } else {
        evaluation
    };
    // The landed file opens with its frontmatter, whatever the lints said.
    if let Err(reason) = candidate::landed_shape(&candidate) {
        let manifest =
            candidate::stage_shape_refusal(staging_root, gate_envelope, call_id, &path, reason)?;
        println!("{}", serde_json::to_string(&manifest)?);
        return Ok(());
    }
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        "land-task-body",
        evaluation,
        vec![crate::command::workflow_gate_envelope::StagedGateOutput {
            relative_path: file_name,
            bytes: candidate,
        }],
    )?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

/// Issue-44: the fidelity section of the body gate. Runs only once the body
/// is mechanically clean, so the critic is never spent on a body the author
/// is already being sent back for. A candidate that reached here parsed, so
/// a parse failure inside the audit is operational, like every other failure.
async fn audit_candidate_fidelity(
    cwd: &Path,
    path: &Path,
    candidate: &[u8],
    call_id: &str,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    mut evaluation: crate::command::workflow_gate::GateEvaluation,
) -> crate::command::workflow_gate::GateEvaluation {
    let tasks_root = path.parent().unwrap_or(path);
    let waivers = crate::command::topology_lint::recorded_waivers(cwd, tasks_root);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        )
        .without_project_tools();
    let client = factory
        .build_client(archon_workflow::WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-decompose-land-task-body".into(),
            session_id: call_id.to_string(),
            read_roots: Vec::new(),
        })
        .await
        .map_err(anyhow::Error::new);
    let outcome = match std::str::from_utf8(candidate) {
        Ok(raw) => {
            crate::command::topology_lint::audit_task_file_candidate(
                cwd, path, raw, client, &waivers,
            )
            .await
        }
        Err(error) => Err(anyhow::Error::new(error).context("candidate body is not UTF-8")),
    };
    match outcome {
        Ok((report, findings)) => {
            evaluation.report.push_str(&report);
            evaluation.findings.extend(findings);
            evaluation
        }
        Err(error) => {
            // Issue 338: as in the set gate.
            crate::command::workflow_host_command_operational::exit_if_unsettled_publish(&error);
            let evaluation = evaluation.with_operational_error(format!(
                "obligation fidelity audit failed operationally: {error:#}"
            ));
            crate::command::topology_lint::tag_unusable_critic_reply(evaluation, &error)
        }
    }
}

#[cfg(test)]
#[path = "workflow_staged_candidate_tests.rs"]
mod candidate_tests;

#[cfg(test)]
#[path = "workflow_staged_candidate_routing_tests.rs"]
mod candidate_routing_tests;
