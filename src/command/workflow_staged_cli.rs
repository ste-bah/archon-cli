//! Hidden trusted-child composition for fixed-decomposition lint capabilities.

use std::io::Read;
use std::path::Path;

use anyhow::{Result, anyhow};

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_staged_task_set_lint(
    cwd: &Path,
    task_file: Option<&Path>,
    tasks: Option<&Path>,
    spec_file: Option<&Path>,
    graph: Option<&str>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    mode: archon_core::config::GateMode,
) -> Result<()> {
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
    let evaluation = match crate::command::topology_lint::evaluate_lint(cwd, &source, mode) {
        Ok(evaluation) => evaluation,
        Err(error) => crate::command::workflow_gate::GateEvaluation::new("", Vec::new())
            .with_operational_error(error.to_string()),
    };
    let staging_root = gate_envelope
        .parent()
        .ok_or_else(|| anyhow!("staged task-set envelope has no parent"))?;
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        cwd,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_staged_task_file_lint(
    cwd: &Path,
    task_file: Option<&Path>,
    tasks: Option<&Path>,
    spec_file: Option<&Path>,
    graph: Option<&str>,
    candidate_stdin: bool,
    staging_root: Option<&Path>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    mode: archon_core::config::GateMode,
) -> Result<()> {
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
    let mut candidate = Vec::new();
    std::io::stdin()
        .take(archon_workflow::HostCommandRequest::MAX_STDIN_BYTES as u64 + 1)
        .read_to_end(&mut candidate)?;
    if candidate.len() > archon_workflow::HostCommandRequest::MAX_STDIN_BYTES {
        return Err(anyhow!(
            "candidate stdin exceeds {} bytes",
            archon_workflow::HostCommandRequest::MAX_STDIN_BYTES
        ));
    }
    let path = if task_file.is_absolute() {
        task_file.to_path_buf()
    } else {
        cwd.join(task_file)
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("candidate TASK path has no UTF-8 file name"))?
        .to_string();
    let evaluation =
        crate::command::topology_lint::evaluate_task_file_candidate(cwd, &path, &candidate, mode)?;
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        cwd,
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
