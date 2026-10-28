//! One supervised physical attempt, fenced by confirmed teardown and sealed evidence.
use super::*;
use crate::command::workflow_host_command_operational::{
    OperationalAttempt, OperationalReport, pause_for_unsettled_publish,
};
use crate::command::workflow_host_command_supervisor::HostCommandControlHandle;

impl FixedHostCommandExecutor {
    /// Failed children still return their canonical sealed envelope, when present.
    pub(super) fn failed_envelope(
        &self,
        fault: Option<archon_workflow::GateOperationalError>,
        anchor: &crate::command::workflow_host_staging_anchor::StagingAnchor,
        path: &std::path::Path,
        secrets: &HostSecrets,
    ) -> WorkflowResult<Option<GateEnvelopeV1>> {
        if let Some(error) = fault {
            return Ok(Some(GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::Value::Null,
                policy_findings: Vec::new(),
                operational_error: Some(error),
            }));
        }
        anchor
            .read_file(ENVELOPE_FILE)
            .map_err(|source| WorkflowError::Io {
                path: path.to_owned(),
                source,
            })?
            .map(|bytes| {
                secrets.parse_json(&bytes, "host command returned malformed gate envelope")
            })
            .transpose()
    }

    /// Issue 386 (item 3): a candidate longer than its capability takes
    /// inline goes to the child through a spill in this attempt's directory,
    /// and the child's stdin carries the bounded view that names it. A
    /// command with no spill directory (no run) keeps the old refusal: there
    /// is nowhere durable to put the candidate.
    pub(super) fn spill_oversized_stdin(
        &self,
        command: &mut ResolvedHostCommand,
    ) -> WorkflowResult<()> {
        use crate::command::workflow_host_command_stdin_spill as spill;
        let Some(stdin) = command.stdin.as_deref() else {
            return Ok(());
        };
        let limit = self
            .catalog
            .capabilities
            .get(&command.command_id)
            .map(|capability| capability.max_stdin_bytes)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "undeclared host command capability '{}'",
                    command.command_id
                ))
            })?;
        if !spill::needs_spill(stdin, limit) {
            return Ok(());
        }
        let Some(directory) = &command.spill_dir else {
            return Err(WorkflowError::SpecInvalid(format!(
                "host command capability '{}' stdin exceeds {limit} bytes and this call has no run directory to spill it to",
                command.command_id
            )));
        };
        let view = spill::spill(directory, stdin)?;
        command.stdin = Some(view);
        Ok(())
    }

    async fn execute_process_with_run_control(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
        handle: HostCommandControlHandle,
        expected_generation: u64,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt(
                    "fixed HostCommand run root has no UTF-8 run id".to_string(),
                )
            })?
            .to_string();
        // A run paused meanwhile reports the pause, never a cancellation.
        crate::command::workflow_host_command_operational::require_run_owned_authorized(
            &store,
            &run_id,
            expected_generation,
            self.paused_run_authorization.as_ref(),
        )?;
        let work = self.process.execute(request, control);
        tokio::pin!(work);
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                biased;
                result = &mut work => return result,
                _ = poll.tick() => {
                    let Ok(run) = store.load_state(&run_id) else {
                        continue;
                    };
                    if let Some(signal) = crate::command::workflow_host_command_operational::supervisor_signal_authorized(
                        &store,
                        &run,
                        expected_generation,
                        self.paused_run_authorization.as_ref(),
                    ) {
                        handle.signal(signal)?;
                        return work.await;
                    }
                }
            }
        }
    }

    /// Supervise one physical attempt. Availability recovery surrounds the whole
    /// unpublished command, while teardown and unsettled-publication fences win.
    pub(super) async fn execute_supervised_attempt(
        &self,
        command: &ResolvedHostCommand,
        call_id: &str,
        expected_generation: u64,
        secrets: &HostSecrets,
        anchor: &crate::command::workflow_host_staging_anchor::StagingAnchor,
        pause: &crate::command::workflow_host_staging_pause::StagingPause,
        teardown: &crate::command::workflow_host_command_teardown_latch::TeardownLatch,
        attempt: u32,
    ) -> WorkflowResult<crate::command::workflow_host_secrets::SealedProcessOutput> {
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("fixed HostCommand run root has no UTF-8 run id".into())
            })?;
        // Every attempt's tree reports to the call's latch, so sealing
        // after a cancellation waits for it (#297 round 8).
        let (control, handle) = HostCommandControl::tracked(teardown.clone());
        let started = std::time::Instant::now();
        // A call may be executed again after resume, when its attempt count
        // starts over. Keep durable spills from separate executions disjoint.
        let mut attempt_command = command.clone();
        if let Some(directory) = &command.spill_dir {
            attempt_command.spill_dir =
                Some(directory.join(format!("attempt-{attempt}-{}", uuid::Uuid::new_v4())));
        }
        self.spill_oversized_stdin(&mut attempt_command)?;
        let observed = self
            .execute_process_with_run_control(attempt_command, control, handle, expected_generation)
            .await;
        let confirmed = crate::command::workflow_host_retry::controlled(
            (
                &store,
                run_id,
                expected_generation,
                self.paused_run_authorization.as_ref(),
            ),
            async { Ok(teardown.wait_confirmed().await) },
        )
        .await
        .map_err(|error| {
            error
                .downcast::<WorkflowError>()
                .unwrap_or_else(|error| WorkflowError::StageFailed(format!("{error:#}")))
        })?;
        if !confirmed {
            return Err(pause.pause(
                anchor.root(),
                "teardown was not confirmed",
                "processes may still write staging; inspect the host command group records",
                true,
            ));
        }
        let evidence = observed
            .as_ref()
            .map(|output| {
                (
                    output.exit_code,
                    crate::command::workflow_host_command_operational::stderr_tail(
                        &output.stderr,
                        secrets,
                    ),
                )
            })
            .unwrap_or_else(|error| {
                (
                    None,
                    crate::command::workflow_host_command_operational::stderr_tail(
                        error.to_string().as_bytes(),
                        secrets,
                    ),
                )
            });
        let history =
            [
                OperationalAttempt::new(attempt, "host_fault", started.elapsed().as_secs(), None)
                    .with_child_evidence(evidence.0, evidence.1),
            ];
        let report = OperationalReport {
            run_id,
            call_id,
            command_id: &command.command_id,
            limit_secs: command.timeout_secs,
            attempts: &history,
        };
        self.require_no_stalled_groups(&store, expected_generation, &report, secrets)?;
        if let Err(error) = anchor.verify() {
            return Err(pause.pause(
                anchor.root(),
                "staging identity could not be verified after the child",
                &secrets.text(&error.to_string()),
                true,
            ));
        }
        let observed = match observed {
            Err(error) => {
                let error = anyhow::Error::new(error);
                if crate::command::workflow_host_fault::classify(&error)
                    == crate::command::workflow_host_fault::Class::Transient
                {
                    crate::command::workflow_host_command_operational::record_retry(
                        &store,
                        &self.run_root,
                        &report,
                    );
                }
                return Err(secrets.error(
                    error
                        .downcast::<WorkflowError>()
                        .expect("workflow error wrapper"),
                ));
            }
            Ok(output) => output,
        };
        let mut observed =
            secrets.seal_process_output(observed, anchor, pause, &command.command_id)?;
        // A child that read nothing over a journal no read could settle
        // already retried the settlement: pause now (Issue 338).
        if let Some(evidence) = &observed.unsettled_publish {
            return Err(pause_for_unsettled_publish(
                &store,
                &self.run_root,
                expected_generation,
                &report,
                &secrets.text(evidence),
            ));
        }
        // A structured child fault takes precedence over its legacy resumable status.
        if let Some(fault) = &observed.fault {
            if fault.kind == crate::command::workflow_host_fault::Class::Transient.kind() {
                let mut evidence = history[0].clone();
                evidence.progress = observed.progress;
                let attempts = [evidence];
                let retry = OperationalReport {
                    attempts: &attempts,
                    ..report
                };
                crate::command::workflow_host_command_operational::record_retry(
                    &store,
                    &self.run_root,
                    &retry,
                );
            }
            return Ok(observed);
        }
        if let Some(kind) = observed.kind {
            if kind == crate::command::workflow_host_command_operational::OperationalKind::TimedOut
            {
                let mut evidence = history[0].clone();
                evidence.progress = observed.progress;
                evidence.reason = kind.label();
                let attempts = [evidence];
                let retry = OperationalReport {
                    attempts: &attempts,
                    ..report
                };
                crate::command::workflow_host_command_operational::record_retry(
                    &store,
                    &self.run_root,
                    &retry,
                );
                let text = format!(
                    "host command {} ended {} under its existing {}s no-progress window: {}",
                    command.command_id,
                    kind.label(),
                    command.timeout_secs,
                    String::from_utf8_lossy(&observed.output.stderr)
                );
                return Err(WorkflowError::port(
                    crate::command::workflow_host_fault::Fault {
                        class: crate::command::workflow_host_fault::Class::Transient,
                        text,
                    },
                ));
            }
            // A status alone proves no availability cause. Legacy/unknown text
            // is permanent and retains exactly the child's diagnostic.
            observed.fault = Some(archon_workflow::GateOperationalError {
                kind: crate::command::workflow_host_fault::Class::Permanent
                    .kind()
                    .into(),
                text: String::from_utf8_lossy(&observed.output.stderr).into_owned(),
            });
        }
        Ok(observed)
    }
}
