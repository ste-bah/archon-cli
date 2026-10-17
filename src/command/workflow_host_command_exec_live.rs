//! The fixed executor's live test for a recorded host command, in two
//! strengths: reuse (accepted, no findings) and liveness (findings allowed:
//! the record's outcome is still exactly what is on disk). A resumed run asks
//! the second question about the last landing of a subject, so a freeze the
//! judge gave findings to is not re-judged while its artifact is untouched.
use super::*;

const PIPELINE_ORDER: &[(&str, usize)] = &[
    ("freeze-acceptance", 0),
    ("freeze-skeleton", 1),
    ("land-task-body", 2),
    ("task-set-lint", 3),
    ("requirements-trace", 4),
    ("verify-frozen-acceptance", 0),
    ("verify-frozen-skeleton", 1),
];

fn pipeline_order(
    catalog: &archon_workflow::CommandCapabilityCatalog,
    command_id: &str,
) -> Option<usize> {
    // This execution order is runtime policy. Keeping it outside the serialized
    // catalog preserves catalog shape and launch identity across binary upgrades.
    catalog
        .capabilities
        .contains_key(command_id)
        .then_some(())?;
    pipeline_stage(command_id)
}

/// A host command's position in the decomposition pipeline: a later stage
/// may extend an earlier stage's shared output, never the reverse.
pub(crate) fn pipeline_stage(command_id: &str) -> Option<usize> {
    PIPELINE_ORDER
        .iter()
        .find_map(|(id, position)| (*id == command_id).then_some(*position))
}

fn record_status_allows_reuse(record: &WorkflowV2CallRecord, findings_allowed: bool) -> bool {
    record.call.method == archon_workflow::WorkflowV2HostMethod::HostCommand
        && record.invalidated_by.is_none()
        && if findings_allowed {
            matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Accepted
                    | archon_workflow::WorkflowV2Status::Noop
                    | archon_workflow::WorkflowV2Status::NeedsReview
            )
        } else {
            matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Accepted
                    | archon_workflow::WorkflowV2Status::Noop
            )
        }
}

fn receipt_entry_extends(
    candidate: &archon_workflow::PublishedArtifactReceipt,
    destination: &str,
    prior_times: &std::collections::BTreeMap<String, String>,
    committed_at: &str,
    live_digest: &str,
    live_len: u64,
) -> bool {
    candidate.destination_path == destination
        && candidate
            .prior_blake3
            .as_ref()
            .and_then(|prior| prior_times.get(prior))
            .is_some_and(|prior_time| committed_at > prior_time.as_str())
        && (candidate.blake3 != live_digest || candidate.byte_len == live_len)
}

/// The command ran to completion and published; findings are not held
/// against it here.
fn landed(outcome: &HostCommandResult) -> bool {
    outcome.exit_code == Some(0)
        && outcome.publication_receipt.is_some()
        && outcome
            .gate_envelope
            .as_ref()
            .is_some_and(|envelope| envelope.operational_error.is_none())
        && outcome
            .postcondition
            .as_ref()
            .is_some_and(|postcondition| postcondition.satisfied)
        && !outcome.timed_out
        && !outcome.interrupted
        && !outcome.stdout_truncated
        && !outcome.stderr_truncated
}

impl FixedHostCommandExecutor {
    pub(super) async fn repair_landing_mismatch_live(
        &self,
        record: &WorkflowV2CallRecord,
        expected_generation: Option<u64>,
        allow_restore: bool,
    ) -> WorkflowResult<(bool, Option<HostCommandResult>, bool)> {
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("landing record has no host command request".into())
        })?;
        let context = self.context_for_request(request)?;
        let (live, restored, superseded) =
            crate::command::workflow_decompose_landing_reconcile::repair_landing_mismatch(
                &context.project_root,
                &self.run_root,
                &context.task_root,
                record,
                allow_restore,
            )
            .map_err(|error| {
                WorkflowError::HostOperational(format!("landing mismatch repair failed: {error:#}"))
            })?;
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

    pub(super) fn landing_receipt_matches_disk_live(
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
            return super::super::workflow_host_command_postcondition::receipt_matches_live(
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
                "landing receipt mismatch for subject {}: landing call {}, receipt blake3 {}, current disk blake3 {}; resume-start landing reconciliation owns restoring a receipt-verified body",
                target.task_id, record.call.id, receipt_digest, current_digest
            )));
        }
        Ok(true)
    }

    pub(super) async fn rejudge_landing_without_publication(
        &self,
        record: &WorkflowV2CallRecord,
        expected_generation: Option<u64>,
        allow_restore: bool,
    ) -> WorkflowResult<Option<super::LandingRejudgeOutcome>> {
        let mut request =
            record.call.options.host_command.clone().ok_or_else(|| {
                WorkflowError::StateCorrupt("landing record has no request".into())
            })?;
        let outcome: HostCommandResult = serde_json::from_value(record.result.data.clone())?;
        let receipt = outcome.publication_receipt.ok_or_else(|| {
            WorkflowError::StateCorrupt("current landing has no publication receipt".into())
        })?;
        let context = self.context_for_request(&request)?;
        let target = request.target.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("current landing has no subject target".into())
        })?;
        let path = context.task_root.join(&target.file_name);
        #[cfg(test)]
        crate::command::workflow_test_seams::apply_before_landing_rejudge(&record.call.id)
            .map_err(|source| WorkflowError::Io {
                path: path.clone(),
                source,
            })?;
        // Settle again immediately before judging. A writer may have changed
        // the file after the earlier pre-judge settle; in that case the
        // newest bytes returned by settlement are the candidate to judge.
        let (bytes, receipt_bytes, superseded) =
            crate::command::workflow_decompose_landing_reconcile::repair_landing_mismatch(
                &context.project_root,
                &self.run_root,
                &context.task_root,
                record,
                allow_restore,
            )
            .map_err(|error| {
                WorkflowError::HostOperational(format!("landing settle failed: {error:#}"))
            })?;
        if superseded {
            return Ok(None);
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        request.stdin = Some(String::from_utf8(bytes).map_err(|error| {
            WorkflowError::ArtifactInvalid(format!(
                "current landing {} is not UTF-8: {error}",
                path.display()
            ))
        })?);
        if receipt_bytes.is_none() {
            // Settlement says these are the newest outside bytes. Run them
            // through the ordinary landing path for a candidate verdict.
            return self
                .execute(request, expected_generation)
                .await
                .map(|outcome| {
                    Some(super::LandingRejudgeOutcome {
                        outcome,
                        candidate: true,
                    })
                });
        }
        let outcome = self
            .execute_inner(request, expected_generation, Some(receipt), true)
            .await?;
        #[cfg(test)]
        crate::command::workflow_test_seams::apply_after_landing_rejudge(&record.call.id).map_err(
            |source| WorkflowError::Io {
                path: path.clone(),
                source,
            },
        )?;
        Ok(Some(super::LandingRejudgeOutcome {
            outcome,
            candidate: false,
        }))
    }

    pub(super) fn record_is_reusable_live(
        &self,
        record: &WorkflowV2CallRecord,
        findings_allowed: bool,
    ) -> WorkflowResult<bool> {
        if !record_status_allows_reuse(record, findings_allowed) {
            return Ok(false);
        }
        // Issue 361: judged by other logic, it is not this build's answer.
        if !self.outcome_logic_holds_for(record)? {
            return Ok(false);
        }
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("persisted HostCommand record has no typed request".into())
        })?;
        if !crate::command::workflow_host_command_occurrence::record_identity_matches(
            record,
            &self.call_identity(request)?,
        ) {
            return Ok(false);
        }
        let outcome: HostCommandResult = serde_json::from_value(record.result.data.clone())?;
        let outcome_ok = if findings_allowed {
            landed(&outcome)
        } else {
            outcome.reusable()
        };
        let receipt_matches = self.receipt_matches_live_with_successors(
            record,
            outcome.publication_receipt.as_ref(),
            findings_allowed,
        )?;
        if !outcome_ok || !receipt_matches {
            return Ok(false);
        }
        let context = match self.context_for_request(request) {
            Ok(context) => context,
            // Nothing the host could not bind is reusable.
            Err(WorkflowError::SpecInvalid(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
        let (_, current_postcondition) = evaluate_postcondition(&context, &request.command_id)?;
        if !current_postcondition.satisfied {
            return Ok(false);
        }
        if findings_allowed {
            // Terminality comes from the record itself: a landed outcome with a
            // satisfied postcondition is terminal for this subject. The state
            // file's disposition is not consulted, because a resume replays
            // every record in script order and a refused attempt replayed just
            // before this one leaves it reading `Pending`.
            return Ok(true);
        }
        fixed_subject_is_terminal(&self.run_root, &request.command_id, &outcome)
    }
}

impl FixedHostCommandExecutor {
    fn receipt_matches_live_with_successors(
        &self,
        original: &WorkflowV2CallRecord,
        receipt: Option<&archon_workflow::PublicationReceiptV1>,
        findings_allowed: bool,
    ) -> WorkflowResult<bool> {
        let Some(receipt) = receipt else {
            return Ok(false);
        };
        if super::super::workflow_host_command_postcondition::receipt_matches_live(Some(receipt))? {
            return Ok(true);
        }
        let records = archon_workflow::WorkflowV2ResultStore::new(self.run_root.join("v2"))
            .load_call_records()?;
        for entry in &receipt.entries {
            let path = std::path::PathBuf::from(&entry.destination_path);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(source) => return Err(WorkflowError::Io { path, source }),
            };
            let live_digest = archon_workflow::task_set_contract::content_digest(&bytes);
            if live_digest == entry.blake3 {
                if bytes.len() as u64 != entry.byte_len {
                    return Ok(false);
                }
                continue;
            }
            let original_request =
                original.call.options.host_command.as_ref().ok_or_else(|| {
                    WorkflowError::StateCorrupt(
                        "persisted HostCommand record has no typed request".into(),
                    )
                })?;
            let Some(original_order) = pipeline_order(&self.catalog, &original_request.command_id)
            else {
                return Ok(false);
            };
            let mut reachable = std::collections::BTreeMap::from([(
                entry.blake3.clone(),
                receipt.committed_at.clone(),
            )]);
            loop {
                let prior_count = reachable.len();
                for candidate_record in &records {
                    let Some(request) = candidate_record.call.options.host_command.as_ref() else {
                        continue;
                    };
                    let Some(candidate_order) = pipeline_order(&self.catalog, &request.command_id)
                    else {
                        continue;
                    };
                    if candidate_order <= original_order
                        || candidate_record.call.id == original.call.id
                    {
                        continue;
                    }
                    let Ok(candidate_outcome) = serde_json::from_value::<HostCommandResult>(
                        candidate_record.result.data.clone(),
                    ) else {
                        continue;
                    };
                    let Some(next) = candidate_outcome.publication_receipt.as_ref() else {
                        continue;
                    };
                    if next.schema_version != archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION
                        || next.command_id != request.command_id
                        || next.call_id != candidate_record.call.id
                    {
                        continue;
                    }
                    let context = match self.context_for_request(request) {
                        Ok(context) => context,
                        Err(WorkflowError::SpecInvalid(_)) => continue,
                        Err(error) => return Err(error),
                    };
                    let declared = match self
                        .resolved(request, &context, &candidate_record.call.id)
                        .and_then(|resolved| {
                            self.destinations(&context, &resolved, &candidate_record.call.id)
                        }) {
                        Ok(destinations) => destinations.into_values().collect::<Vec<_>>(),
                        Err(WorkflowError::SpecInvalid(_)) => continue,
                        Err(error) => return Err(error),
                    };
                    if !declared.iter().any(|declared| declared == &path) {
                        continue;
                    }
                    if !self.record_is_reusable_live(candidate_record, findings_allowed)? {
                        continue;
                    }
                    for candidate in &next.entries {
                        if !receipt_entry_extends(
                            candidate,
                            &entry.destination_path,
                            &reachable,
                            &next.committed_at,
                            &live_digest,
                            bytes.len() as u64,
                        ) {
                            continue;
                        }
                        reachable.insert(candidate.blake3.clone(), next.committed_at.clone());
                    }
                }
                if reachable.contains_key(&live_digest) || reachable.len() == prior_count {
                    break;
                }
            }
            if !reachable.contains_key(&live_digest) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Issue 337: what a call of `request` judges, in the context the host
    /// binds it to. A candidate the host cannot bind judged no content.
    pub(super) fn judged_inputs_for(
        &self,
        request: &HostCommandRequest,
    ) -> WorkflowResult<Option<String>> {
        match self.context_for_request(request) {
            Ok(context) => {
                crate::command::workflow_host_command_judged_inputs::judged_inputs_digest(
                    &context, request,
                )
            }
            Err(WorkflowError::SpecInvalid(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
#[path = "workflow_host_command_exec_live_tests.rs"]
mod successor_reuse_tests;
