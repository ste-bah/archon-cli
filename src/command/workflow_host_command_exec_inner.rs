use super::*;

impl FixedHostCommandExecutor {
    pub(super) async fn execute_inner(
        &self,
        mut request: HostCommandRequest,
        expected_generation: Option<u64>,
        existing_receipt: Option<archon_workflow::PublicationReceiptV1>,
        judge_only: bool,
    ) -> WorkflowResult<HostCommandResult> {
        let expected_generation = expected_generation.ok_or_else(|| {
            WorkflowError::StateCorrupt(
                "fixed HostCommand execution has no dispatch generation".into(),
            )
        })?;
        crate::command::workflow_host_command_integrity::require_launch_prd_unchanged(
            &self.context,
            &request.command_id,
        )?;
        // A body the host cannot bind to a frozen subject is the author's
        // artifact being wrong, not the host failing: it belongs in the findings
        // channel that gives the author its next attempt, exactly like a
        // candidate the gate refuses.
        let bound = super::super::workflow_host_command_binding::bind_request(
            &self.context,
            &self.run_root,
            &request,
        );
        let context = match bound {
            Ok(Ok(context)) => context,
            Ok(Err(unbound)) => return unbound.refused(&request.command_id),
            Err(WorkflowError::SpecInvalid(reason)) => {
                return Ok(unpublished(
                    Some(0),
                    String::new(),
                    String::new(),
                    (0, 0),
                    candidate_refusal_envelope(&request.command_id, &reason),
                    "candidate refused before staging",
                ));
            }
            Err(error) => return Err(error),
        };
        let call_id = self.content_identity(&request, &context)?;
        // A receipt-backed body has already been stamped at landing. Its stdin
        // is the live body whose exact bytes were checked against that receipt;
        // stamping again could change those bytes before private staging.
        let stamped_fields = if existing_receipt.is_some() || judge_only {
            Vec::new()
        } else {
            match stamp::stamp_request(&mut request, &context)? {
                Ok(notes) => notes,
                Err(unstampable) => return unstampable.refused(&request.command_id),
            }
        };
        let pause = super::super::workflow_host_staging_pause::StagingPause::new(
            &self.context.project_root,
            &self.run_root,
            expected_generation,
            &call_id,
            &request.command_id,
        )?;
        // Stale staging a cancelled call left is cleared, or the run pauses (#297).
        let staging = pause.prepare(&self.run_root)?;
        let mut command = self.resolved(&request, &context, &call_id)?;
        if judge_only
            && matches!(
                command.command_id.as_str(),
                "verify-frozen-acceptance" | "verify-frozen-skeleton"
            )
        {
            command.args.push("--read-only".into());
        }
        // What the call records never holds a secret value the child was given.
        let secrets = Arc::new(HostSecrets::of(&context, &command.environment));
        let pause = Arc::new(pause);
        let teardown = TeardownLatch::default();
        let staged_envelope = staging.root.join(ENVELOPE_FILE);
        let mut cleanup = EnvelopeCleanup {
            anchor: staging.anchor.clone(),
            secrets: secrets.clone(),
            pause: pause.clone(),
            teardown: teardown.clone(),
            remove_after_teardown: Vec::new(),
            armed: true,
        };
        if (existing_receipt.is_some() || judge_only)
            && command.args.windows(2).any(|pair| pair[0] == "--task-file")
        {
            let task_file = context.frozen_task_file.as_ref().ok_or_else(|| {
                WorkflowError::StateCorrupt(
                    "judge-only body execution has no frozen task file".into(),
                )
            })?;
            let file_name = task_file
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    WorkflowError::StateCorrupt(
                        "receipt-backed body re-judge task file has no UTF-8 name".into(),
                    )
                })?;
            let task_bytes = request.stdin.as_deref().ok_or_else(|| {
                WorkflowError::StateCorrupt(
                    "judge-only body execution has no candidate bytes".into(),
                )
            })?;
            if let Err(source) = staging.anchor.write_file(file_name, task_bytes.as_bytes()) {
                if let Err(cleanup_error) =
                    staging.anchor.remove_file(std::path::Path::new(file_name))
                    && cleanup_error.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(pause.pause(
                        &staging.root.join(file_name),
                        "temporary task body staging could not be removed after write failure",
                        &cleanup_error.to_string(),
                        true,
                    ));
                }
                return Err(WorkflowError::Io {
                    path: staging.root.join(file_name),
                    source,
                });
            }
            cleanup
                .remove_after_teardown
                .push(std::path::PathBuf::from(file_name));
            let task_file_arg = command
                .args
                .windows(2)
                .position(|pair| pair[0] == "--task-file")
                .ok_or_else(|| {
                    WorkflowError::StateCorrupt(
                        "judge-only body catalog has no --task-file argument".into(),
                    )
                })?;
            command.args[task_file_arg + 1] =
                staging.root.join(file_name).to_string_lossy().into_owned();
        }
        for path in &command.declared_write_set {
            let contained =
                resolved_path_is_within_root(&staging.root, path).map_err(|source| {
                    WorkflowError::Io {
                        path: path.clone(),
                        source,
                    }
                })?;
            if !contained {
                return Err(WorkflowError::SpecInvalid(
                    "resolved host command write set escaped prepared staging root".to_string(),
                ));
            }
        }
        let destinations = self.destinations(&context, &command, &call_id)?;
        let sentinels =
            LiveMutationSentinels::capture(&destinations.values().cloned().collect::<Vec<_>>())
                .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
        let result = async {
            // Issue #255: an operational ending is retried or pauses the run.
            let sealed = self
                .execute_with_operational_retry(
                    &command,
                    &call_id,
                    expected_generation,
                    &secrets,
                    &staging.anchor,
                    &pause,
                    &teardown,
                )
                .await?;
            let truncated = sealed.truncated;
            let prepared = sealed.prepared;
            let observed = sealed.output;
            if existing_receipt.is_some() {
                sentinels
                    .verify()
                    .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            }
            let stdout = utf8(observed.stdout, "stdout")?;
            let stderr = utf8(observed.stderr, "stderr")?;
            if observed.exit_code != Some(0) {
                return Ok(HostCommandResult {
                    exit_code: observed.exit_code,
                    stdout,
                    stderr,
                    stdout_bytes: observed.stdout_bytes,
                    stderr_bytes: observed.stderr_bytes,
                    timed_out: observed.timed_out,
                    interrupted: false,
                    stdout_truncated: truncated.0,
                    stderr_truncated: truncated.1,
                    gate_envelope: None,
                    publication_receipt: None,
                    subjects: Vec::new(),
                    postcondition: None,
                    observed_only: false,
                });
            }
            let Some(prepared) = prepared else {
                return Err(WorkflowError::StateCorrupt(
                    "successful host call has no manifest".into(),
                ));
            };
            let staged = staging.anchor.read_file(ENVELOPE_FILE).and_then(|bytes| {
                bytes.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "the staged envelope is not a regular file",
                    )
                })
            });
            let envelope: GateEnvelopeV1 = secrets.parse_json(
                &staged.map_err(|source| WorkflowError::Io {
                    path: staged_envelope.clone(),
                    source,
                })?,
                "host command returned malformed gate envelope",
            )?;
            let envelope = secrets.envelope(envelope);
            if envelope.policy_findings.iter().any(|finding| {
                !command
                    .remediation_scopes
                    .contains(&finding.remediation_scope)
            }) {
                return Err(WorkflowError::PolicyDenied(format!(
                    "host command '{}' returned a remediation scope outside its catalog",
                    request.command_id
                )));
            }
            if envelope.operational_error.is_some() {
                // The script stops on `operational_error` itself, and it can only do
                // that if the envelope reaches it. Failing the call here instead
                // discarded the envelope and left the bridge with an error it could
                // not render as an outcome, so the operator saw a conversion
                // complaint rather than the reason the phase stopped.
                return Ok(unpublished(
                    observed.exit_code,
                    stdout,
                    stderr,
                    (observed.stdout_bytes, observed.stderr_bytes),
                    envelope,
                    "host gate reported an operational failure",
                ));
            }
            if judge_only || existing_receipt.is_some() {
                // The current landing is already on disk. Rejudging it must
                // never publish or rewrite that artifact, even when the new
                // verdict contains findings.
                let (subjects, postcondition) =
                    evaluate_postcondition(&context, &command.command_id)?;
                let (stdout, stdout_bytes) =
                    append_host_note(stdout, &stamped_fields, observed.stdout_bytes);
                return Ok(HostCommandResult {
                    exit_code: observed.exit_code,
                    stdout,
                    stderr,
                    stdout_bytes,
                    stderr_bytes: observed.stderr_bytes,
                    timed_out: false,
                    interrupted: false,
                    stdout_truncated: truncated.0,
                    stderr_truncated: truncated.1,
                    gate_envelope: Some(envelope),
                    publication_receipt: existing_receipt.clone(),
                    subjects,
                    postcondition: Some(postcondition),
                    observed_only: judge_only,
                });
            }
            if candidate_refused_before_staging(&prepared, &command) {
                return Ok(unpublished(
                    observed.exit_code,
                    stdout,
                    stderr,
                    (observed.stdout_bytes, observed.stderr_bytes),
                    envelope,
                    "candidate refused before staging",
                ));
            }
            if candidate_findings_prevent_publication(
                &command.command_id,
                context.gate_mode,
                &envelope,
            ) {
                return Ok(unpublished(
                    observed.exit_code,
                    stdout,
                    stderr,
                    (observed.stdout_bytes, observed.stderr_bytes),
                    envelope,
                    "candidate findings prevented parent publication",
                ));
            }
            let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
            let run_id = self
                .run_root
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    WorkflowError::StateCorrupt(
                        "fixed HostCommand run root has no UTF-8 run id".into(),
                    )
                })?
                .to_string();
            let shadow_root = self.context.project_root.clone();
            let pin = super::super::workflow_task_set::acceptance_pin_path(
                &context.project_root,
                &context.task_root,
            );
            let committed = store.with_run_lock(&run_id, |locked| {
                // A sibling stopped by a pause reports "paused", not "cancelled".
                crate::command::workflow_host_command_operational::require_run_owned_locked_authorized(
                    locked,
                    &run_id,
                    expected_generation,
                    self.paused_run_authorization.as_ref(),
                )?;
                // The audit reads by path: refuse a tree swapped since creation.
                staging.anchor.verify().map_err(|error| {
                    WorkflowError::ArtifactInvalid(format!("host command staging refused: {error}"))
                })?;
                let audited = audit_prepared_publication(&staging, &prepared, &command, sentinels)
                    .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
                let receipt = match publish_audited(audited, &destinations, &pin, &context.task_root) {
                    Ok(receipt) => receipt,
                    Err(error)
                        if error
                            .downcast_ref::<super::super::workflow_task_set::StaleAcceptanceContract>()
                            .is_some() => return Ok(None),
                    Err(error) => return Err(WorkflowError::StageFailed(error.to_string())),
                };
                if let Some(envelope) = destinations.get(ENVELOPE_FILE) {
                    owner_only(envelope)?;
                }
                // Parent-only recording follows every publication refusal.
                // Never fatal here. The publication is already committed to the
                // live tree; failing the call now would lose the receipt the script
                // needs while leaving the commit in place - the partial state this
                // whole path exists to prevent.
                if let Err(error) = crate::command::workflow_gate::append_published_shadow_records(
                    &shadow_root,
                    &call_id,
                    &command.command_id,
                    &envelope.policy_findings,
                    "staged",
                ) {
                    tracing::warn!(%error, "recording published gate findings failed");
                }
                let (subjects, postcondition) =
                    evaluate_postcondition(&context, &command.command_id)?;
                Ok(Some((receipt, subjects, postcondition)))
            })?;
            let Some((receipt, subjects, postcondition)) = committed else {
                // Read whole under the chain's read lock: the loop rebuilds
                // its candidate from exactly the contract now published
                // (`null` when none is).
                let current = {
                    let _read = crate::command::workflow_task_set::ChainRead::workflow(
                        &context.project_root,
                        &context.task_root,
                    )?;
                    let path = context
                        .task_root
                        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
                    match std::fs::read(&path) {
                        Ok(bytes) => serde_json::from_slice(&bytes)?,
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                            serde_json::Value::Null
                        }
                        Err(source) => return Err(WorkflowError::Io { path, source }),
                    }
                };
                let mut stale = stale_acceptance_contract(
                    observed.exit_code,
                    stdout,
                    stderr,
                    (observed.stdout_bytes, observed.stderr_bytes),
                    envelope,
                    current,
                );
                stale.gate_envelope = stale.gate_envelope.map(|envelope| secrets.envelope(envelope));
                return Ok(stale);
            };
            // Through the anchor, after the lock: a failure is recorded.
            finish::remove_published(&staging, &receipt, &pause, &secrets);
            let (stdout, stdout_bytes) =
                append_host_note(stdout, &stamped_fields, observed.stdout_bytes);
            Ok(HostCommandResult {
                exit_code: observed.exit_code,
                stdout,
                stderr,
                stdout_bytes,
                stderr_bytes: observed.stderr_bytes,
                timed_out: false,
                interrupted: false,
                stdout_truncated: truncated.0,
                stderr_truncated: truncated.1,
                gate_envelope: Some(envelope),
                publication_receipt: Some(receipt),
                subjects,
                postcondition: Some(postcondition),
                observed_only: false,
            })
        }
        .await;
        #[cfg(test)]
        if let Some(hook) = AFTER_CALL.take() {
            hook(&staging.root);
        }
        let sealed = cleanup.finish();
        finish::settle_cleanup(result, sealed, &staging.root, &pause, &secrets)
            .map_err(|error| secrets.error(error))
    }
}

/// Compare filesystem identities rather than path spellings. On Windows the
/// same directory can be named through an 8.3 alias or a verbatim path.
fn resolved_path_is_within_root(
    root: &std::path::Path,
    path: &std::path::Path,
) -> std::io::Result<bool> {
    fn resolve(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        if path.exists() {
            return archon_shell::paths::canonicalize(path);
        }
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
        })?;
        let name = path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
        })?;
        Ok(archon_shell::paths::canonicalize(parent)?.join(name))
    }

    resolved_path_is_within_root_using(root, path, resolve)
}

fn resolved_path_is_within_root_using(
    root: &std::path::Path,
    path: &std::path::Path,
    resolve: impl Fn(&std::path::Path) -> std::io::Result<std::path::PathBuf>,
) -> std::io::Result<bool> {
    let canonical_root = resolve(root)?;
    let canonical_path = resolve(path)?;
    Ok(canonical_path.starts_with(canonical_root))
}

#[cfg(test)]
mod prepared_write_path_tests {
    use super::resolved_path_is_within_root_using;
    use std::path::{Path, PathBuf};

    #[test]
    fn staging_containment_uses_resolved_path_identity() {
        // EXPECTED BEFORE CHECK: the short-name spelling resolves to the
        // staging directory and must be contained; a sibling must not be.
        let root = Path::new(r"C:\Users\runneradmin\AppData\Local\Temp\stage");
        let short_child = Path::new(r"C:\Users\RUNNER~1\AppData\Local\Temp\stage\gate.json");
        let outside = Path::new(r"C:\Users\RUNNER~1\AppData\Local\Temp\stage-other\gate.json");
        let resolve = |path: &Path| -> std::io::Result<PathBuf> {
            let value = path
                .to_string_lossy()
                .replace("RUNNER~1", "runneradmin")
                .replace('\\', "/");
            Ok(PathBuf::from(value))
        };
        assert!(!short_child.starts_with(root));
        assert!(resolved_path_is_within_root_using(root, short_child, resolve).unwrap());
        let resolve = |path: &Path| -> std::io::Result<PathBuf> {
            let value = path
                .to_string_lossy()
                .replace("RUNNER~1", "runneradmin")
                .replace('\\', "/");
            Ok(PathBuf::from(value))
        };
        assert!(!resolved_path_is_within_root_using(root, outside, resolve).unwrap());
    }
}
