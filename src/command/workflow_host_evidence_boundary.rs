//! The ingress boundary for child streams, manifests, retry evidence and errors.
//! Only sealed output can leave this module for persistence or publication.
use super::HostSecrets;
use crate::command::{
    workflow_host_command_operational::{
        OperationalKind, classify, reported_progress, unsettled_publish_evidence,
    },
    workflow_host_command_supervisor::SupervisedProcessOutput,
    workflow_host_envelope_seal::{ENVELOPE_FILE, refuse_staged_evidence, seal_staged_envelope},
    workflow_host_staging_anchor::StagingAnchor,
    workflow_host_staging_pause::StagingPause,
};
use archon_workflow::{PreparedPublicationV1, WorkflowError, WorkflowResult};
use std::path::Path;

pub(crate) struct SealedProcessOutput {
    pub(crate) output: SupervisedProcessOutput,
    pub(crate) prepared: Option<PreparedPublicationV1>,
    pub(crate) truncated: (bool, bool),
    pub(crate) kind: Option<OperationalKind>,
    pub(crate) progress: Option<u64>,
    pub(crate) unsettled_publish: Option<String>,
}
impl HostSecrets {
    pub(crate) fn seal_process_output(
        &self,
        mut output: SupervisedProcessOutput,
        anchor: &StagingAnchor,
        pause: &StagingPause,
        command_id: &str,
    ) -> WorkflowResult<SealedProcessOutput> {
        // Control facts are interpreted before scrubbing. Credentials can
        // collide with protocol markers without turning a stall into success.
        let kind = classify(&output);
        let progress = reported_progress(&output.stderr)
            .filter(|value| !self.holds_serialized_secret(value.to_string().as_bytes()));
        let unsettled_publish = unsettled_publish_evidence(output.exit_code, &output.stderr)
            .map(|evidence| self.text(&evidence));
        let truncated = output.truncation();
        if kind.is_none() {
            for (stream, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
                std::str::from_utf8(bytes).map_err(|error| {
                    WorkflowError::StageFailed(format!(
                        "host command {stream} is not UTF-8: {error}"
                    ))
                })?;
            }
        }
        // The raw manifest is internal integrity evidence only. Verify its
        // staged identity before rebinding to the canonical sealed envelope.
        let mut prepared = if kind.is_none() && output.exit_code == Some(0) {
            Some(self.parse_json::<PreparedPublicationV1>(
                &output.stdout,
                &format!("host command '{command_id}' returned malformed prepared manifest"),
            )?)
        } else {
            None
        };
        self.seal_staged_evidence(anchor, prepared.as_mut(), pause)?;
        if let Some(manifest) = &prepared {
            output.stdout = serde_json::to_vec(manifest)?;
        }
        output.stdout = self
            .text(&String::from_utf8_lossy(&output.stdout))
            .into_bytes();
        output.stderr = self
            .text(&String::from_utf8_lossy(&output.stderr))
            .into_bytes();
        Ok(SealedProcessOutput {
            output,
            prepared,
            truncated,
            kind,
            progress,
            unsettled_publish,
        })
    }

    /// Seals the call's staging. On a failure the whole tree is removed;
    /// a removal failure or an I/O failure pauses the run naming the path
    /// (#297), and only an integrity refusal (a dishonest manifest) fails.
    pub(crate) fn seal_staged_evidence(
        &self,
        anchor: &StagingAnchor,
        mut prepared: Option<&mut PreparedPublicationV1>,
        pause: &StagingPause,
    ) -> WorkflowResult<()> {
        let seal: WorkflowResult<()> = (|| {
            seal_staged_envelope(anchor, self, prepared.as_deref_mut())?;
            if self.remove_unsafe_artifacts(anchor)? {
                refuse_staged_evidence(anchor, prepared.as_deref_mut())?;
                if let Some(prepared) = prepared {
                    // Do not persist raw digests of refused secret-bearing files.
                    prepared
                        .entries
                        .retain(|entry| entry.relative_path == ENVELOPE_FILE);
                }
            }
            Ok(())
        })();
        let Err(error) = seal else {
            return Ok(());
        };
        let evidence = self.text(&error.to_string());
        match anchor.remove_tree() {
            Err(cleanup) => Err(pause.pause(
                anchor.root(),
                "refused because secret-bearing staging could not be removed",
                &format!(
                    "{evidence}; removing it failed: {}",
                    self.text(&cleanup.to_string())
                ),
                true,
            )),
            Ok(()) if matches!(error, WorkflowError::Io { .. }) => Err(pause.pause(
                anchor.root(),
                "staging could not be sealed (it was removed)",
                &evidence,
                false,
            )),
            Ok(()) => Err(error),
        }
    }

    /// Signed/non-envelope artifacts cannot be rewritten without invalidating
    /// their contracts. Remove unsafe staging and return a canonical operational
    /// refusal before any publication, using the same secret verification.
    fn remove_unsafe_artifacts(&self, anchor: &StagingAnchor) -> WorkflowResult<bool> {
        anchor
            .scan(true, &mut |relative, bytes| {
                relative != Path::new(ENVELOPE_FILE)
                    && bytes.is_some_and(|bytes| self.holds_serialized_secret(bytes))
            })
            .map_err(|source| WorkflowError::Io {
                path: anchor.root().into(),
                source,
            })
    }

    /// Retain control/operational classification while scrubbing all error
    /// data. This also covers integrity errors quoting child manifest fields.
    pub(crate) fn error(&self, error: WorkflowError) -> WorkflowError {
        if self.text(&error.to_string()) == error.to_string() {
            return error;
        }
        match error {
            WorkflowError::ControlPaused(text) => WorkflowError::ControlPaused(self.text(&text)),
            WorkflowError::ControlCancelled(text) => {
                WorkflowError::ControlCancelled(self.text(&text))
            }
            WorkflowError::HostOperational(text) => {
                WorkflowError::HostOperational(self.text(&text))
            }
            WorkflowError::HostCallTimeout(text) => {
                WorkflowError::HostCallTimeout(self.text(&text))
            }
            WorkflowError::TerminalHostCall(text) => {
                WorkflowError::TerminalHostCall(self.text(&text))
            }
            WorkflowError::StageFailed(text) => WorkflowError::StageFailed(self.text(&text)),
            WorkflowError::ArtifactInvalid(text) => {
                WorkflowError::ArtifactInvalid(self.text(&text))
            }
            WorkflowError::PolicyDenied(text) => WorkflowError::PolicyDenied(self.text(&text)),
            WorkflowError::SpecInvalid(text) => WorkflowError::SpecInvalid(self.text(&text)),
            WorkflowError::StateCorrupt(text) => WorkflowError::StateCorrupt(self.text(&text)),
            WorkflowError::Io { path, source } => WorkflowError::Io {
                path: self.text(&path.to_string_lossy()).into(),
                source: std::io::Error::new(source.kind(), self.text(&source.to_string())),
            },
            error => WorkflowError::StageFailed(self.text(&error.to_string())),
        }
    }
}
