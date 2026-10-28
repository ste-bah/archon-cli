//! Safe setup failures retain their typed availability cause for recovery.
//! Unsafe residue or unconfirmed teardown leaves the run
//! paused, never failed, and the pause names the staging path. When the call
//! no longer owns the run (an operator paused or cancelled it, or a resume
//! superseded it) that decision stands, but staging that may still hold child
//! output is recorded anyway, so the next resume, which clears the same path
//! before its child runs, finds it named.
use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowError, WorkflowEventKind, WorkflowResult, WorkflowStore};

use super::workflow_host_command_operational::{append_log, emit};
use super::workflow_host_staging_residue::StagingResidue;

pub(crate) struct StagingPause {
    store: WorkflowStore,
    run_root: PathBuf,
    run_id: String,
    generation: u64,
    call_id: String,
    command_id: String,
    /// Written before the child can write staging, removed once it is
    /// sealed (#297 round 9).
    residue: StagingResidue,
}

impl StagingPause {
    pub(crate) fn new(
        project_root: &Path,
        run_root: &Path,
        generation: u64,
        call_id: &str,
        command_id: &str,
    ) -> WorkflowResult<Self> {
        let run_id = run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("fixed HostCommand run root has no UTF-8 run id".into())
            })?;
        Ok(Self {
            store: WorkflowStore::project(project_root),
            run_root: run_root.to_path_buf(),
            run_id: run_id.to_string(),
            generation,
            call_id: call_id.to_string(),
            command_id: command_id.to_string(),
            residue: StagingResidue::at(run_root, call_id),
        })
    }

    /// The call's staging, created empty under its anchor, after its residue
    /// record. Stale staging that cannot be cleared,
    /// pauses the run naming it. Safe residue-record I/O retains its typed
    /// availability cause so the host can retry it before any child runs.
    pub(crate) fn prepare(
        &self,
        run_root: &Path,
    ) -> WorkflowResult<super::workflow_host_command_publish::CommandStaging> {
        use super::workflow_host_command_publish::{prepare_staging, staging_root};
        (|| {
            #[cfg(test)]
            crate::command::workflow_host_fault_test_hooks::fire(
                crate::command::workflow_host_fault_test_hooks::Point::Prepare,
                run_root,
            )?;
            self.residue
                .record(&self.call_id, &self.command_id, self.generation)
        })()
        .map_err(|source| WorkflowError::Io {
            path: self.residue.path().to_path_buf(),
            source,
        })?;
        // The killed attempt's partial staging must not reach the
        // audit, which requires the staged set to match the manifest.
        // Cleared through the anchor: a link the killed attempt left
        // is removed, never followed, and the run pauses when it cannot be.
        prepare_staging(run_root, &self.call_id).map_err(|error| {
            if !crate::command::workflow_host_retry::transient(&error)
                && std::fs::symlink_metadata(staging_root(run_root, &self.call_id)).is_ok()
            {
                return self.pause(
                    &staging_root(run_root, &self.call_id),
                    "existing staging could not be safely cleared",
                    &format!("{error:#}"),
                    true,
                );
            }
            if crate::command::workflow_host_retry::transient(&error) {
                // No child has run. Confirm removal through a fresh anchor before retrying.
                if super::workflow_host_staging_anchor::StagingAnchor::clear(
                    run_root,
                    staging_root(run_root, &self.call_id)
                        .file_name()
                        .expect("staging identity")
                        .to_str()
                        .expect("staging identity"),
                )
                .is_ok()
                {
                    return crate::command::workflow_host_fault::workflow(
                        error,
                        format!(
                            "preparing staging {}",
                            staging_root(run_root, &self.call_id).display()
                        ),
                    );
                }
            } else {
                return crate::command::workflow_host_fault::workflow(
                    error,
                    format!(
                        "preparing staging {}",
                        staging_root(run_root, &self.call_id).display()
                    ),
                );
            }
            self.pause(
                &staging_root(run_root, &self.call_id),
                "staging could not be safely prepared",
                &format!("{error:#}"),
                true,
            )
        })
    }

    /// The call's staging is sealed and no process of the call can write it:
    /// its residue record goes. A record that stays only makes the next
    /// resume remove staging that is already sealed.
    pub(crate) fn sealed(&self) {
        if let Err(error) = self.residue.clear() {
            let path = self.residue.path().display().to_string();
            tracing::warn!(%error, %path, "host command staging residue record not removed");
        }
    }

    /// Records, without pausing, that `step` left staging at `path` (the
    /// call's outcome is already committed). The next prepare clears it.
    pub(crate) fn record_residue(&self, path: &Path, step: &str, evidence: &str) {
        let path = path.display().to_string();
        tracing::warn!(run_id = %self.run_id, %path, "host command {step}: {evidence}");
        let detail = serde_json::json!({
            "event": "host_command_staging_residue",
            "call_id": self.call_id,
            "command_id": self.command_id,
            "path": path,
            "step": step,
            "evidence": evidence,
            "residue": true,
        });
        if let Err(error) = emit(
            &self.store,
            &self.run_id,
            WorkflowEventKind::StageStalled,
            detail,
        ) {
            tracing::error!(%error, %path, "host command staging residue not recorded");
        }
    }

    /// Pauses the run because `step` failed at `path` with `evidence` (already
    /// scrubbed of credentials). `residue`: the staging may still hold child
    /// output, so the path is recorded even when the pause is refused.
    pub(crate) fn pause(
        &self,
        path: &Path,
        step: &str,
        evidence: &str,
        residue: bool,
    ) -> WorkflowError {
        let resume = format!("archon workflow resume --live --yes {}", self.run_id);
        let path = path.display().to_string();
        let message = format!(
            "host command '{}' (call {}) {step} at '{path}': {evidence}; the run is paused, not failed; repair the staging and resume: {resume}",
            self.command_id, self.call_id
        );
        let mut detail = serde_json::json!({
            "event": "host_command_staging_pause",
            "call_id": self.call_id,
            "command_id": self.command_id,
            "path": path,
            "step": step,
            "evidence": evidence,
            "residue": residue,
            "resume": resume,
        });
        let paused = archon_workflow::control_pause::pause_with_evidence(
            &self.store,
            &self.run_id,
            self.generation,
            detail.clone(),
        );
        match paused {
            Ok(event) => {
                tracing::warn!(run_id = %self.run_id, "{message}");
                match event {
                    Ok(seq) => append_log(
                        &self.run_root,
                        &format!(
                            "event_id={seq} transition=host_command_staging_pause call_id={} command_id={} residue={residue} next_action=resume run_id={}",
                            self.call_id, self.command_id, self.run_id
                        ),
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "host command staging pause event not recorded")
                    }
                }
                WorkflowError::ControlPaused(message)
            }
            Err(refused) => {
                if residue {
                    detail["event"] = "host_command_staging_residue".into();
                    detail["refused"] = refused.to_string().into();
                    if let Err(error) = emit(
                        &self.store,
                        &self.run_id,
                        WorkflowEventKind::StageStalled,
                        detail,
                    ) {
                        tracing::error!(%error, %path, "host command staging residue not recorded");
                    }
                }
                match refused {
                    WorkflowError::ControlPaused(why) => {
                        WorkflowError::ControlPaused(format!("{why}; {message}"))
                    }
                    WorkflowError::ControlCancelled(why) => {
                        WorkflowError::ControlCancelled(format!("{why}; {message}"))
                    }
                    other => other,
                }
            }
        }
    }
}
