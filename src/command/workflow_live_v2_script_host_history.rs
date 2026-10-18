//! History replay for a resumed fixed run: see
//! `archon_workflow::v2::script::history_replay`.
use super::*;
use archon_workflow::v2::script::history_replay::{replayable_answer, replayable_history};

#[derive(serde::Serialize, serde::Deserialize)]
struct LandingRejudgeObservation {
    schema_version: u32,
    landing_call_id: String,
    receipt_digest: String,
    logic_version: Option<u32>,
    logic_digest: Option<String>,
    logic_build: Option<String>,
    outcome: Option<archon_workflow::HostCommandResult>,
}

impl LandingRejudgeObservation {
    fn key(
        executor: &dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<(String, Self)> {
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("landing record has no host command request".into())
        })?;
        let outcome: archon_workflow::HostCommandResult =
            serde_json::from_value(record.result.data.clone())?;
        let receipt = outcome.publication_receipt.ok_or_else(|| {
            WorkflowError::StateCorrupt("current landing has no publication receipt".into())
        })?;
        let receipt_bytes = serde_json::to_vec(&receipt)?;
        let receipt_digest = archon_workflow::task_set_contract::content_digest(&receipt_bytes);
        let observation = Self {
            schema_version: 1,
            landing_call_id: record.call.id.clone(),
            receipt_digest,
            logic_version: executor.logic_version(request)?,
            logic_digest: executor.logic_digest(request)?,
            logic_build: executor.logic_build(request)?,
            outcome: None,
        };
        let key_data = serde_json::json!({
            "landing_call_id": observation.landing_call_id,
            "receipt_digest": observation.receipt_digest,
            "logic_version": observation.logic_version,
            "logic_digest": observation.logic_digest,
            "logic_build": observation.logic_build,
        });
        let key =
            archon_workflow::task_set_contract::content_digest(&serde_json::to_vec(&key_data)?);
        Ok((key, observation))
    }
}

impl WorkflowScriptHost {
    /// The recorded result of a call that a later call over the same subject
    /// has superseded, when this is a fixed run resuming and the call arrives
    /// with the input it was recorded with. Its status is irrelevant: a
    /// refused or malformed attempt, or a freeze that carried findings, is
    /// history exactly as an accepted one is, and replaying it verbatim is
    /// what keeps the phase's budget and best artifact the same across a
    /// pause. A record a pause or cancel interrupted is no answer and never
    /// history. Agent calls and host commands alike; the live paths below never
    /// see a superseded record. A run seeded after an upgrade has no such
    /// history from before its seed (`predates_phase_seed`).
    pub(super) async fn replay_superseded_history(
        &self,
        execution: &WorkflowV2CallExecution,
        input_hash: &str,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        if self.runner.host_command_executor.is_none() {
            return Ok(None);
        }
        let Some(record) = self.runner.v2_store.load_call_record(&execution.call.id)? else {
            return Ok(None);
        };
        let records = self.runner.v2_store.load_call_records()?;
        // Batch H: history answers only the question it was recorded for.
        if self.answer_predates_question(execution, &record)? {
            return Ok(None);
        }
        // Issue 360: a seeded run replays no attempt recorded before its seed;
        // a landing the executor finds still live is the artifact on disk.
        let history = !self.predates_phase_seed(&record)?
            && replayable_history(&record, &records, input_hash);
        if !history && !self.landed_record_still_on_disk(&record, input_hash)? {
            return Ok(None);
        }
        if !self.outcome_holds(&record)? {
            return Ok(None);
        }
        if !self.refresh_audit_for_cache(&record).await? {
            return Ok(None);
        }
        self.mark_reused(&record, generation).await?;
        Ok(Some(match generation {
            Some(generation) => self.result_view_in_generation(&record, generation)?,
            None => self.result_view(&record)?,
        }))
    }

    /// A landing request already answered is history, except an unpublished
    /// refusal judged by older logic or the request of its subject's current
    /// body (`archon_workflow::v2::script::landing_lineage`). A refusal is
    /// rejudged only if no later round was accepted or published.
    pub(super) async fn replay_historic_landing(
        &self,
        execution: &mut WorkflowV2CallExecution,
        generation: Option<u64>,
        force_fresh_execution: &mut bool,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        use archon_workflow::v2::script::landing_lineage::{
            LandingLineage, call_occurrence, subject_of,
        };
        let (Some(executor), Some(request)) = (
            self.runner.host_command_executor.as_ref(),
            execution.call.options.host_command.as_ref(),
        ) else {
            return Ok(None);
        };
        if execution.call.method != WorkflowV2HostMethod::HostCommand
            || !executor.publishes(request)?
        {
            return Ok(None);
        }
        let Some(subject) = subject_of(request) else {
            return Ok(None);
        };
        let records = self.runner.v2_store.load_call_records()?;
        let lineage = LandingLineage::of(&records, &request.command_id, subject);
        let store = &self.runner.v2_store;
        let occurrence = call_occurrence(&execution.call);
        let matched_answer = lineage.superseded_answer(request, occurrence, |record| {
            store.in_session(&record.call.id)
        });
        let used_missing_stdin_fallback = matched_answer.is_none();
        let record = matched_answer.or_else(|| {
            lineage.answers().iter().copied().find(|record| {
                !lineage.is_rerun(record)
                    && call_occurrence(&record.call) == occurrence
                    && record
                        .call
                        .options
                        .host_command
                        .as_ref()
                        .is_some_and(|recorded| {
                            recorded.stdin.is_none()
                                && recorded.command_id == request.command_id
                                && recorded.target == request.target
                        })
            })
        });
        let Some(record) = record else {
            return Ok(None);
        };
        let later_accepted_or_published = lineage
            .answers()
            .iter()
            .position(|answer| answer.call.id == record.call.id)
            .is_some_and(|index| {
                lineage.answers()[index + 1..].iter().any(|later| {
                    matches!(
                        later.status,
                        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
                    ) || later.result.data["publicationReceipt"].is_object()
                })
            });
        if used_missing_stdin_fallback
            && !(archon_workflow::v2::script::landing_lineage::landing_refused(&record)
                && record.result.data["publicationReceipt"].is_null()
                && !later_accepted_or_published
                && !executor.outcome_logic_holds(&record)?)
        {
            return Ok(None);
        }
        if archon_workflow::v2::script::landing_lineage::landing_refused(&record) {
            let outcome: archon_workflow::HostCommandResult =
                serde_json::from_value(record.result.data.clone())?;
            if outcome.publication_receipt.is_none() && !later_accepted_or_published {
                let recorded_request =
                    record.call.options.host_command.clone().ok_or_else(|| {
                        WorkflowError::StateCorrupt(
                            "superseded landing refusal has no recorded host command request"
                                .into(),
                        )
                    })?;
                if !executor.outcome_logic_holds(&record)? {
                    if recorded_request.stdin.is_none() {
                        return Err(WorkflowError::StateCorrupt(format!(
                            "superseded landing refusal {} has no recorded candidate bytes",
                            record.call.id
                        )));
                    }
                    execution.call.options.host_command = Some(recorded_request);
                    *force_fresh_execution = true;
                    tracing::info!(
                        call_id = %execution.call.id,
                        recorded_call_id = %record.call.id,
                        "superseded refusal is being judged again under current host command logic"
                    );
                    return Ok(None);
                }
            }
        }
        if matches!(
            record.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) && !archon_workflow::v2::script::landing_lineage::landing_refused(&record)
            && let Some(current) = lineage.current().cloned()
        {
            // Reconcile a displaced file under the frozen chain lock before
            // deciding which bytes can receive a verdict.
            let mut receipt_restore_used = false;
            match executor.landing_receipt_matches_disk(&current) {
                Ok(true) => {}
                Ok(false) | Err(WorkflowError::HostOperational(_)) => {
                    let (restored, candidate, superseded) = executor
                        .repair_landing_mismatch(&current, generation, true)
                        .await?;
                    if superseded {
                        let records = self.runner.v2_store.load_call_records()?;
                        let latest =
                            archon_workflow::v2::script::landing_lineage::LandingLineage::of(
                                &records,
                                &request.command_id,
                                subject,
                            )
                            .current()
                            .ok_or_else(|| {
                                WorkflowError::StateCorrupt(format!(
                                    "subject {subject} lost its current landing during repair"
                                ))
                            })?;
                        return Ok(Some(match generation {
                            Some(generation) => {
                                self.result_view_in_generation(latest, generation)?
                            }
                            None => self.result_view(latest)?,
                        }));
                    }
                    if let Some(outcome) = candidate {
                        let answer = self.rejudged_answer(&current, outcome)?;
                        let view = match generation {
                            Some(generation) => {
                                self.result_view_in_generation(&answer, generation)?
                            }
                            None => self.result_view(&answer)?,
                        };
                        return Ok(Some(view));
                    }
                    if !restored {
                        // No disk bytes and no rebuildable receipt: let the
                        // normal author path obtain a fresh body.
                        return Ok(None);
                    }
                    receipt_restore_used = true;
                }
                Err(error) => return Err(error),
            }
            // The accepted historical answer is stale evidence for this
            // subject. Reuse the current landing's verdict when its logic
            // still holds; otherwise judge the receipt-verified disk bytes
            // under this build without publishing them.
            let current = current;
            if !executor.outcome_logic_holds(&current)? {
                let (observation_key, observation) =
                    LandingRejudgeObservation::key(executor.as_ref(), &current)?;
                if let Some(saved) = self
                    .runner
                    .v2_store
                    .load_landing_rejudge_observation::<LandingRejudgeObservation>(
                        &observation_key,
                    )?
                {
                    if saved.schema_version != 1
                        || saved.landing_call_id != observation.landing_call_id
                        || saved.receipt_digest != observation.receipt_digest
                        || saved.logic_version != observation.logic_version
                        || saved.logic_digest != observation.logic_digest
                        || saved.logic_build != observation.logic_build
                    {
                        return Err(WorkflowError::StateCorrupt(format!(
                            "landing rejudge observation {observation_key} does not match its content key"
                        )));
                    }
                    let mut outcome = saved.outcome.ok_or_else(|| {
                        WorkflowError::StateCorrupt(format!(
                            "landing rejudge observation {observation_key} has no judgement"
                        ))
                    })?;
                    let landing_outcome: archon_workflow::HostCommandResult =
                        serde_json::from_value(current.result.data.clone())?;
                    outcome.publication_receipt = landing_outcome.publication_receipt;
                    let answer = self.rejudged_answer(&current, outcome)?;
                    tracing::info!(
                        landing_call_id = %current.call.id,
                        observation_key,
                        "replayed current landing's judge-only observation"
                    );
                    return Ok(Some(match generation {
                        Some(generation) => self.result_view_in_generation(&answer, generation)?,
                        None => self.result_view(&answer)?,
                    }));
                }
                let judged = executor
                    .rejudge_landing(&current, generation, !receipt_restore_used)
                    .await?;
                let Some(judged) = judged else {
                    return Ok(None);
                };
                if judged.candidate {
                    let answer = self.rejudged_answer(&current, judged.outcome)?;
                    return Ok(Some(match generation {
                        Some(generation) => self.result_view_in_generation(&answer, generation)?,
                        None => self.result_view(&answer)?,
                    }));
                }
                let outcome = judged.outcome;
                let receipt_matches = match executor.landing_receipt_matches_disk(&current) {
                    Ok(true) => true,
                    Ok(false) | Err(WorkflowError::HostOperational(_)) => false,
                    Err(error) => return Err(error),
                };
                if !receipt_matches {
                    let (_restored, candidate, superseded) = executor
                        // A judge has already run in this pass. Treat any
                        // newer bytes as its own candidate rather than
                        // restoring the receipt and judging stale bytes.
                        .repair_landing_mismatch(&current, generation, false)
                        .await?;
                    if superseded {
                        let records = self.runner.v2_store.load_call_records()?;
                        let latest =
                            archon_workflow::v2::script::landing_lineage::LandingLineage::of(
                                &records,
                                &request.command_id,
                                subject,
                            )
                            .current()
                            .ok_or_else(|| {
                                WorkflowError::StateCorrupt(format!(
                                    "subject {subject} lost its current landing during repair"
                                ))
                            })?;
                        return Ok(Some(match generation {
                            Some(generation) => {
                                self.result_view_in_generation(latest, generation)?
                            }
                            None => self.result_view(latest)?,
                        }));
                    }
                    if let Some(outcome) = candidate {
                        let answer = self.rejudged_answer(&current, outcome)?;
                        return Ok(Some(match generation {
                            Some(generation) => {
                                self.result_view_in_generation(&answer, generation)?
                            }
                            None => self.result_view(&answer)?,
                        }));
                    }
                    // Let ordinary execution route this subject back through
                    // its author when no bytes exist and the receipt cannot
                    // be rebuilt.
                    return Ok(None);
                }
                let mut persisted_observation = observation;
                persisted_observation.outcome = Some(outcome.clone());
                if let Some(outcome_without_publication) = persisted_observation.outcome.as_mut() {
                    outcome_without_publication.publication_receipt = None;
                }
                self.persist_landing_rejudge_observation(&observation_key, persisted_observation)?;
                let answer = self.rejudged_answer(&current, outcome)?;
                return Ok(Some(match generation {
                    Some(generation) => self.result_view_in_generation(&answer, generation)?,
                    None => self.result_view(&answer)?,
                }));
            }
            tracing::info!(
                call_id = %execution.call.id,
                current_call_id = %current.call.id,
                subject,
                "accepted superseded landing answered with the current landing verdict"
            );
            return Ok(Some(match generation {
                Some(generation) => self.result_view_in_generation(&current, generation)?,
                None => self.result_view(&current)?,
            }));
        }
        tracing::info!(
            call_id = %execution.call.id,
            answered_by = %record.call.id,
            subject,
            "landing request is a superseded round; its recorded answer is replayed and nothing is published"
        );
        self.mark_reused(&record, generation).await?;
        Ok(Some(match generation {
            Some(generation) => self.result_view_in_generation(&record, generation)?,
            None => self.result_view(&record)?,
        }))
    }

    fn rejudged_answer(
        &self,
        landing: &WorkflowV2CallRecord,
        outcome: archon_workflow::HostCommandResult,
    ) -> archon_workflow::WorkflowResult<WorkflowV2CallRecord> {
        let status = if outcome.reusable() {
            WorkflowV2Status::Accepted
        } else {
            WorkflowV2Status::NeedsReview
        };
        let mut answer = landing.clone();
        answer.status = status;
        answer.result.status = status;
        answer.result.data =
            crate::command::workflow_host_command_outcome_stamp::stamp_host_outcome(
                self.runner.host_command_executor.as_deref(),
                &landing.call,
                serde_json::to_value(outcome)?,
            )?;
        answer.result.summary = "current landing rejudged under this build".into();
        Ok(answer)
    }

    fn persist_landing_rejudge_observation(
        &self,
        observation_key: &str,
        observation: LandingRejudgeObservation,
    ) -> archon_workflow::WorkflowResult<()> {
        self.runner
            .v2_store
            .save_landing_rejudge_observation(observation_key, &observation)
    }

    /// Issue-250: a reused record read from the call's history (its last
    /// accepted record, displaced by a later attempt that a pause, a cancel
    /// or a dead host interrupted) goes back into the call's slot before it
    /// is credited, so every reader of the slot sees the answer that was
    /// reused. Always under the run lock, the lock every restart holds, and
    /// only while no restart has moved the restart epoch on since this
    /// session opened (any run kind, Issue-256) and, for a fixed run, while
    /// `generation` still owns the run. A slot record reused as it stands
    /// is left alone.
    pub(in super::super) fn restore_reused_record(
        &self,
        record: &WorkflowV2CallRecord,
        from_history: bool,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<()> {
        if !from_history {
            return Ok(());
        }
        let run_id = &self.runner.run_id;
        self.runner.workflow_store.with_run_lock(run_id, |locked| {
            if let Some(expected) = generation {
                let current = locked.load_state(run_id)?.generation;
                if current != expected {
                    return Err(WorkflowError::ControlCancelled(format!(
                        "generation {expected} cannot restore the accepted record of call {} for run {run_id}; current generation is {current}",
                        record.call.id
                    )));
                }
            }
            // Issue 291: the epoch, and this session's executor.
            self.runner.v2_store.require_session_owner()?;
            self.runner.v2_store.restore_call_record(record)
        })?;
        tracing::info!(
            call_id = %record.call.id,
            attempt = record.attempt,
            "last accepted record restored from the call's history"
        );
        Ok(())
    }

    /// The last landing of a subject is not history, but it is not a question
    /// to ask again either while the executor finds it live -- identity,
    /// receipt, postcondition and terminal subject all still holding: its
    /// findings were the judge's answer about exactly this artifact, and only
    /// a non-deterministic judge would answer differently. Re-asking is how a
    /// resumed run lost an accepted contract. A receipt that no longer matches
    /// (an operator edit during the pause) falls through to live execution.
    fn landed_record_still_on_disk(
        &self,
        record: &WorkflowV2CallRecord,
        input_hash: &str,
    ) -> archon_workflow::WorkflowResult<bool> {
        if record.call.method != WorkflowV2HostMethod::HostCommand
            || !replayable_answer(record, input_hash)
            || record.result.data["publicationReceipt"].is_null()
        {
            return Ok(false);
        }
        let Some(executor) = self.runner.host_command_executor.as_ref() else {
            return Ok(false);
        };
        executor.record_is_live(record)
    }
}
