//! Staged-command output for the freeze CLI: the envelope manifest a staged
//! freeze prints, the refusals and operational failures it reports through
//! that envelope, and its bounded candidate stdin. Split out of
//! `workflow_freeze_cli.rs` to hold the 500-line ceiling.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

use super::StagedArgs;

/// Report a staged command that failed operationally through its envelope.
///
/// The host owns the reason a phase stopped. Exiting non-zero throws that
/// reason away: the parent sees no envelope, the script reports the *absence*
/// of a receipt, and the operator is told "no committed publication receipt"
/// when the truth was a truncated judge response or an unreadable pin. The
/// envelope already carries `operational_error` and the script already stops on
/// it, so the honest failure travels the channel built for it.
pub(super) fn report_operational_failure(
    cwd: &Path,
    staged: StagedArgs<'_>,
    command_id: &str,
    error: &anyhow::Error,
) -> Result<()> {
    // Issue 338: a set no read can settle is the run's pause, not the stage's.
    crate::command::workflow_host_command_operational::exit_if_unsettled_publish(error);
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
pub(super) fn refuse_candidate_artifact(
    cwd: &Path,
    staged: StagedArgs<'_>,
    command_id: &str,
    gate_id: crate::command::workflow_gate::GateId,
    subject: &str,
    code: &str,
    reason: &str,
) -> Result<()> {
    // The caller names the code, so the refusal's stage is the host's own
    // classification, never a guess from its message.
    let finding = crate::command::workflow_gate::GateFinding::new(
        gate_id,
        format!(
            "candidate artifact was refused: {reason}. Return the artifact alone as raw JSON, with no prose, commentary or code fences, matching the required shape exactly."
        ),
        subject,
        None,
        archon_workflow::RemediationScope::CandidateArtifact,
    )
    .with_defect(archon_workflow::defect::DeterministicDefect::new(
        code,
        subject,
        "candidate",
    ));
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

pub(super) fn write_staged_manifest(
    _cwd: &Path,
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

pub(super) fn read_bounded_stdin(limit: usize) -> Result<Vec<u8>> {
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
