//! The ingress boundary for child streams, manifests, retry evidence and errors.
//! Only sealed output can leave this module for persistence or publication.
use super::HostSecrets;
use crate::command::{
    workflow_host_command_operational::{
        OperationalKind, classify, reported_progress, unsettled_publish_evidence,
    },
    workflow_host_command_supervisor::SupervisedProcessOutput,
    workflow_host_envelope_seal::{refuse_staged_evidence, seal_staged_envelope},
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
        envelope: &Path,
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
        self.seal_staged_evidence(envelope, prepared.as_mut())?;
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

    pub(crate) fn seal_staged_evidence(
        &self,
        envelope: &Path,
        mut prepared: Option<&mut PreparedPublicationV1>,
    ) -> WorkflowResult<()> {
        let root = envelope.parent().expect("staged envelope has a root");
        let seal = (|| {
            seal_staged_envelope(envelope, self, prepared.as_deref_mut())?;
            if self.remove_unsafe_artifacts(root, envelope)? {
                refuse_staged_evidence(envelope, prepared.as_deref_mut())?;
                if let Some(prepared) = prepared {
                    // Do not persist raw digests of refused secret-bearing files.
                    prepared.entries.retain(|entry| {
                        entry.relative_path
                            == crate::command::workflow_host_envelope_seal::ENVELOPE_FILE
                    });
                }
            }
            Ok(())
        })();
        if let Err(error) = seal {
            if let Err(cleanup) = Self::remove_staging_tree(root) {
                return Err(WorkflowError::ControlPaused(format!(
                    "host command refused because secret-bearing staging could not be removed at '{}': {cleanup}",
                    root.display()
                )));
            }
            return Err(error);
        }
        Ok(())
    }

    /// Restore directory traversal and write access before removing the entire
    /// call tree. A child may have changed permissions after writing secrets.
    fn remove_staging_tree(root: &Path) -> std::io::Result<()> {
        fn make_accessible(path: &Path) -> std::io::Result<()> {
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Ok(());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            #[cfg(not(unix))]
            {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                std::fs::set_permissions(path, permissions)?;
            }
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    make_accessible(&entry.path())?;
                }
            }
            Ok(())
        }

        match std::fs::symlink_metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return std::fs::remove_file(root);
            }
            Ok(_) => {}
        }
        make_accessible(root)?;
        std::fs::remove_dir_all(root)
    }

    /// Signed/non-envelope artifacts cannot be rewritten without invalidating
    /// their contracts. Remove unsafe staging and return a canonical operational
    /// refusal before any publication, using the same secret verification.
    fn remove_unsafe_artifacts(&self, root: &Path, envelope: &Path) -> WorkflowResult<bool> {
        let io = |path: &Path, source| WorkflowError::Io {
            path: path.into(),
            source,
        };
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(io(root, error)),
        };
        let mut refused = false;
        for entry in entries {
            let entry = entry.map_err(|error| io(root, error))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|error| io(&path, error))?;
            if kind.is_dir() {
                refused |= self.remove_unsafe_artifacts(&path, envelope)?;
            } else if kind.is_file() && path != envelope {
                let bytes = std::fs::read(&path).map_err(|error| io(&path, error))?;
                if self.holds_serialized_secret(&bytes) {
                    std::fs::remove_file(&path).map_err(|error| io(&path, error))?;
                    refused = true;
                }
            }
        }
        Ok(refused)
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
