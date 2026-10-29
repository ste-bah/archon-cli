//! The frozen stage a `verify-frozen-chain` call names.

use super::verify_frozen_stage_with_recovery;
use anyhow::{Result, anyhow};
use std::path::Path;

/// The stage a `verify-frozen-chain` call re-verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrozenStage {
    Acceptance,
    Skeleton,
}

impl FrozenStage {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        match text {
            "acceptance" => Ok(Self::Acceptance),
            "skeleton" => Ok(Self::Skeleton),
            other => Err(anyhow!(
                "verify-frozen-chain stage must be acceptance or skeleton, found '{other}'"
            )),
        }
    }

    pub(crate) fn command_id(self) -> &'static str {
        match self {
            Self::Acceptance => "verify-frozen-acceptance",
            Self::Skeleton => "verify-frozen-skeleton",
        }
    }
}

/// The `workflow verify-frozen-chain` trusted child behind the
/// `verify-frozen-acceptance` / `verify-frozen-skeleton` capabilities. It
/// publishes nothing: its gate envelope is the whole staged output, and the
/// parent's successful postcondition (the freeze command's own) reads the chain
/// again. A negative answer retains its envelope with an unsatisfied
/// postcondition so the owning author can repair the defective artifact.
/// A stage that does not verify is reported through the envelope, never by
/// exiting non-zero, so the script receives the reason instead of "no committed
/// publication receipt". Authored findings reopen their owner; transient
/// inability and proven corruption retain distinct error kinds for the host.
/// An unsettled publication retains its child exit so the host pauses the run.
pub(crate) fn handle_staged_verify(
    cwd: &Path,
    stage: &str,
    tasks: &Path,
    prd: &Path,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    read_only: bool,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged frozen-chain verification"
        ));
    }
    let manifest = stage_verify(cwd, stage, tasks, prd, gate_envelope, call_id, read_only)?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

/// The staged child's work, minus the print: the envelope under the staging
/// root and the prepared manifest naming it.
pub(crate) fn stage_verify(
    cwd: &Path,
    stage: &str,
    tasks: &Path,
    prd: &Path,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    read_only: bool,
) -> Result<archon_workflow::PreparedPublicationV1> {
    let result = (|| {
        let stage = FrozenStage::parse(stage)?;
        let gate_envelope = gate_envelope.ok_or_else(|| {
            anyhow!("trusted staged verify-frozen-chain requires --gate-envelope <PATH>")
        })?;
        let call_id = call_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("trusted staged verify-frozen-chain requires --call-id <ID>"))?;
        let absolute = |path: &Path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            }
        };
        let evaluation = match verify_frozen_stage_with_recovery(
            cwd,
            &absolute(prd),
            &absolute(tasks),
            stage,
            !read_only,
        ) {
            Ok(snapshot) => crate::command::workflow_gate::GateEvaluation::new(
                if stage == FrozenStage::Skeleton {
                    serde_json::to_string(&snapshot.to_argument())?
                } else {
                    format!(
                        "{} verified: acceptance={}, skeleton={}, {} frozen subject(s), {} body file(s) present",
                        stage.command_id(),
                        snapshot.acceptance,
                        snapshot.skeleton,
                        snapshot.subjects.len(),
                        snapshot.bodies.len()
                    )
                },
                Vec::new(),
            ),
            Err(error) => {
                // Issue 338: a set no read can settle pauses the run instead.
                // Proven journal corruption keeps its permanent envelope and exact remedy.
                if !crate::command::workflow_task_set::UnsettledPublish::permanent(&error) {
                    crate::command::workflow_host_command_operational::exit_if_unsettled_publish(
                        &error,
                    );
                }
                super::verification::evaluation(&error, cwd, &absolute(tasks), stage)
            }
        };
        let staging_root = gate_envelope
            .parent()
            .ok_or_else(|| anyhow!("staged verify-frozen-chain envelope has no parent"))?;
        crate::command::workflow_gate_envelope::stage_gate_evaluation(
            staging_root,
            gate_envelope,
            call_id,
            stage.command_id(),
            evaluation,
            Vec::new(),
        )
    })();
    result.map_err(crate::command::workflow_host_fault::child_failure)
}
