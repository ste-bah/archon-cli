//! Landing reconciliation before availability recovery.
use super::*;
impl FixedHostCommandExecutor {
    pub(in super::super) async fn observe_landing<T>(
        &self,
        generation: Option<u64>,
        mut read: impl FnMut() -> anyhow::Result<T>,
    ) -> WorkflowResult<T> {
        let generation = generation.ok_or_else(|| {
            WorkflowError::StateCorrupt("landing observation has no dispatch generation".into())
        })?;
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("landing observation has no run identity".into())
            })?;
        crate::command::workflow_host_retry::retry(
            "landing-observation",
            (
                &store,
                run_id,
                generation,
                self.paused_run_authorization.as_ref(),
            ),
            || std::future::ready(read()),
        )
        .await
        .map_err(|error| {
            error.downcast::<WorkflowError>().unwrap_or_else(|error| {
                crate::command::workflow_host_fault::workflow(
                    error,
                    "observing landing artifacts".into(),
                )
            })
        })
    }

    pub(in super::super) async fn repair_landing_mismatch_live(
        &self,
        record: &WorkflowV2CallRecord,
        expected_generation: Option<u64>,
        allow_restore: bool,
    ) -> WorkflowResult<(bool, Option<HostCommandResult>, bool)> {
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("landing record has no host command request".into())
        })?;
        let context = self
            .observe_landing(expected_generation, || {
                self.context_for_request(request)
                    .map_err(anyhow::Error::new)
            })
            .await?;
        let (live, restored, superseded) = self
            .observe_landing(expected_generation, || {
                crate::command::workflow_decompose_landing_reconcile::repair_landing_mismatch(
                    &context.project_root,
                    &self.run_root,
                    &context.task_root,
                    record,
                    allow_restore,
                )
            })
            .await?;
        if superseded {
            return Ok((false, None, true));
        }
        if restored.is_some() {
            return Ok((true, None, false));
        }
        if live.is_empty() {
            // No file remains and the receipt cannot reconstruct one. Let
            // history replay fall through to the ordinary body author.
            return Ok((false, None, false));
        }
        let mut candidate = request.clone();
        candidate.stdin = Some(String::from_utf8(live).map_err(|error| {
            WorkflowError::ArtifactInvalid(format!("displaced task body is not UTF-8: {error}"))
        })?);
        let outcome = self.execute(candidate, expected_generation).await?;
        Ok((false, Some(outcome), false))
    }

    pub(in super::super) fn landing_receipt_matches_disk_live(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowResult<bool> {
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand
            || record.invalidated_by.is_some()
            || record.call.options.host_command.is_none()
        {
            return Ok(false);
        }
        if record
            .call
            .options
            .host_command
            .as_ref()
            .is_some_and(|request| request.target.is_none())
        {
            let outcome: HostCommandResult = serde_json::from_value(record.result.data.clone())?;
            return crate::command::workflow_host_command_postcondition::receipt_matches_live(
                outcome.publication_receipt.as_ref(),
            );
        }
        self.require_landing_receipt_matches_disk_live(record)
    }

    fn require_landing_receipt_matches_disk_live(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowResult<bool> {
        // The record may carry an earlier logic-version key by design. The
        // receipt's destination digests, not that historical key, establish
        // whether this is still the subject's current disk artifact.
        let outcome: HostCommandResult = serde_json::from_value(record.result.data.clone())?;
        let receipt = outcome.publication_receipt.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("current landing has no publication receipt".into())
        })?;
        let request =
            record.call.options.host_command.as_ref().ok_or_else(|| {
                WorkflowError::StateCorrupt("landing record has no request".into())
            })?;
        let target = request.target.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("current landing has no subject target".into())
        })?;
        let context = self.context_for_request(request)?;
        let path = context.task_root.join(&target.file_name);
        let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        });
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(WorkflowError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let current_digest = archon_workflow::task_set_contract::content_digest(&bytes);
        let path_key = crate::command::workflow_host_command_publish::path_comparison_key(
            &crate::command::workflow_host_command_publish::path_text(&path),
        );
        let receipt_digest = receipt
            .entries
            .iter()
            .find(|entry| {
                crate::command::workflow_host_command_publish::path_comparison_key(
                    &entry.destination_path,
                ) == path_key
            })
            .map(|entry| entry.blake3.as_str())
            .unwrap_or("missing receipt entry");
        if receipt_digest != current_digest {
            return Err(WorkflowError::HostOperational(format!(
                "landing receipt mismatch for {} (subject {}): landing call {}, receipt blake3 {}, current disk blake3 {}; restore this named body from its authenticated publication receipt",
                path.display(),
                target.task_id,
                record.call.id,
                receipt_digest,
                current_digest
            )));
        }
        Ok(true)
    }
}
