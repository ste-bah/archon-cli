// Composition root for the v3 authored-script lifecycle.
//
// The dialect reference, the authoring bootstrap script, the accounting
// validators and the map→reduce review pre-flight are
// `archon_workflow::v2::script`. What lives here is the part that cannot: an
// inherent `impl WorkflowV2ScriptRunner`, whose every step runs the concrete
// `WorkflowScriptHost` over the live agent client.

use super::*;

/// How many times an authoring attempt may come back with a script the
/// pre-flight rejects before the run gives up.
///
/// Was effectively 2 (one attempt plus one retry). A rejection is a defect the
/// author can learn from — it now receives its own rejected draft and repairs
/// it — so the budget matches `max_repair_iterations`' reasoning rather than
/// being the smallest number that is not one.
const MAX_AUTHORING_DEFECT_ATTEMPTS: usize = 6;

/// How many times the authoring call may die in transport before the run gives
/// up. Separate from the defect budget on purpose: a cancelled or dropped call
/// produced no script, taught the author nothing, and must not consume the
/// chances reserved for actually fixing a defect.
const MAX_AUTHORING_TRANSPORT_ATTEMPTS: usize = 6;

/// A failure that produced no script to learn from, rather than a defective one.
///
/// Cancellations are the case that matters: the subagent layer reports a
/// cancelled task as `join panic: task N was cancelled`, and treating that as
/// an authoring defect burns a retry on a network blip.
fn is_transport_failure(err: &archon_workflow::WorkflowError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    [
        "cancelled",
        "canceled",
        "transport failed",
        "timed out",
        "timeout",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

impl WorkflowV2ScriptRunner {
    /// v3 entry: author workflow.js if absent (journaled, cache-keyed on the composed brief
    /// (task paths + per-file content fingerprints + lessons)), persist it, then execute it. Re-runs with an unchanged
    /// authored script replay unchanged call prefixes from the store.
    pub(in super::super::super) async fn run_authored_script_lifecycle(
        self,
        authored_path: std::path::PathBuf,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let expected_task_ids = self
            .task_universe
            .as_ref()
            .map(|universe| {
                universe
                    .tasks
                    .iter()
                    .map(|task| task.canonical_task_id.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        let authored_source = if authored_path.exists() {
            let source =
                std::fs::read_to_string(&authored_path).map_err(|err| WorkflowError::Io {
                    path: authored_path.clone(),
                    source: err,
                })?;
            let source = validate_authored_workflow_source(&source)?;
            // Pre-flight: the persisted script must still plan real work.
            if let Err(reason) = validate_authored_plan(&source, &expected_task_ids).await {
                return Err(WorkflowError::SpecInvalid(format!(
                    "persisted authored-workflow.js failed its dry-run pre-flight ({reason}); delete {} to re-author",
                    authored_path.display()
                )));
            }
            source
        } else {
            // Author, pre-flight in a dry run, and re-author ONCE with the
            // specific pre-flight error — an authored script that would do no
            // real work must never reach live execution (V3-D1/V3-D2 class).
            // ONE bounded retry covers BOTH failure kinds: a rejected plan
            // AND an unusable authoring envelope (e.g. workflow_js outside
            // data) — each retry names the specific defect.
            // Attempts are budgeted against DEFECTS, not against luck. A
            // transport cancellation is neither an authoring defect nor a
            // reason to end the run, and treating it as one is what killed
            // wf-ac47347c: attempt 1 was rejected for a one-line missing
            // marker, attempt 2 was cancelled 33 seconds in without ever
            // producing a script, and the run died with its single retry
            // spent on a network blip.
            let mut rejection: Option<(String, Option<String>)> = None;
            let mut defect_attempts = 0usize;
            let mut transport_attempts = 0usize;
            let source = loop {
                let (feedback, draft) = match &rejection {
                    Some((reason, draft)) => (Some(reason.as_str()), draft.as_deref()),
                    None => (None, None),
                };
                let authored = self.author_workflow_source(feedback, draft).await;
                match authored {
                    Ok(source) => match validate_authored_plan(&source, &expected_task_ids).await {
                        Ok(()) => break source,
                        Err(reason) => {
                            defect_attempts += 1;
                            if defect_attempts >= MAX_AUTHORING_DEFECT_ATTEMPTS {
                                return Err(WorkflowError::SpecInvalid(format!(
                                    "authored workflow failed its dry-run pre-flight {defect_attempts} times; last error: {reason}"
                                )));
                            }
                            rejection = Some((reason, Some(source)));
                        }
                    },
                    Err(err) if is_transport_failure(&err) => {
                        transport_attempts += 1;
                        if transport_attempts >= MAX_AUTHORING_TRANSPORT_ATTEMPTS {
                            return Err(err);
                        }
                        // The prior rejection (if any) stands: this attempt
                        // produced nothing to learn from, so the next one asks
                        // the same question rather than starting over blind.
                    }
                    Err(err) => {
                        defect_attempts += 1;
                        let reason = format!(
                            "the authoring envelope was unusable ({err}); the complete script text must be the data.workflow_js field of the standard result envelope"
                        );
                        if defect_attempts >= MAX_AUTHORING_DEFECT_ATTEMPTS {
                            return Err(WorkflowError::SpecInvalid(format!(
                                "authored workflow failed its dry-run pre-flight {defect_attempts} times; last error: {reason}"
                            )));
                        }
                        // No usable script came back, so there is no draft to
                        // repair — the next attempt gets the reason alone.
                        rejection = Some((reason, None));
                    }
                }
            };
            std::fs::write(&authored_path, &source).map_err(|err| WorkflowError::Io {
                path: authored_path.clone(),
                source: err,
            })?;
            source
        };
        let summary = self.clone().run(&authored_source).await?;
        let mut review_details = dry_run_workflow_plan_full_details(&authored_source, None).await?;
        review_details.calls = summary.calls.clone();
        validate_map_reduce_review_calls(&review_details, &expected_task_ids).map_err(|reason| {
            WorkflowError::SpecInvalid(format!(
                "the executed run violated the mandatory map→reduce review contract ({reason}); the live call sequence diverged from the pre-flight plan (likely conditional review calls) — delete {} to re-author with unconditional reviews",
                authored_path.display()
            ))
        })?;
        validate_authored_task_accounting(summary.script_result.as_deref(), &expected_task_ids)?;
        validate_review_accounting_from_reducers(
            summary.script_result.as_deref(),
            &review_details,
            &self.v2_store,
        )?;
        Ok(summary)
    }

    pub(super) async fn author_workflow_source(
        &self,
        retry_feedback: Option<&str>,
        rejected_draft: Option<&str>,
    ) -> archon_workflow::WorkflowResult<String> {
        let mut bootstrap = self.clone();
        // Frontier reuse is content-keyed now, so the authoring call needs no
        // opt-out of its own: the brief (task paths + per-file fingerprints +
        // lessons + retry feedback) IS the hashed input, so the retry attempt
        // carries a different hash from the first and can never replay it.
        let (task_paths, source_roots) = self
            .task_universe
            .as_ref()
            .map(|universe| {
                let paths = universe
                    .tasks
                    .iter()
                    .map(|task| {
                        let fingerprint = std::fs::read(&task.source_path)
                            .map(|bytes| {
                                use sha2::{Digest, Sha256};
                                hex::encode(&Sha256::digest(&bytes)[..8])
                            })
                            .unwrap_or_else(|_| "unreadable".to_string());
                        format!(
                            "- {}: {} (fingerprint {fingerprint})",
                            task.canonical_task_id, task.source_path
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (paths, universe.source_roots.join(", "))
            })
            .unwrap_or_default();
        let author_task = compose_author_brief(&[
            (
                "repo_root",
                self.runtime
                    .target_repository_root
                    .as_deref()
                    .unwrap_or("<none>"),
            ),
            ("source_roots", &source_roots),
            ("task_paths", &task_paths),
            // The commands the task authors already verified. Handed over
            // rather than left to the author agent, which has no shell: left to
            // guess, one live run invented a package-wide test filter, drew in
            // failures no task in the universe owned, and could neither satisfy
            // nor abandon them for six hours.
            (
                "declared_focused_tests",
                &self
                    .task_universe
                    .as_ref()
                    // Named in full: the shared import block in
                    // `workflow_live_v2_script` is deliberately explicit, and
                    // this is the only caller here.
                    .map(archon_workflow::v2::script::render_declared_focused_tests)
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
            // Computed here rather than asked of the author. The universe
            // already carries `dependency_ids` and the declared target files,
            // so the batching is determined data, not a judgement — and left as
            // a judgement the author declined it, emitting 60 sequential calls
            // for a task set whose four provider ingests share no dependency
            // and write to four separate directories.
            (
                "task_waves",
                &self
                    .task_universe
                    .as_ref()
                    .map(render_author_waves)
                    .unwrap_or_else(|| "<none>".to_string()),
            ),
            (
                "retry_feedback",
                // The rejected draft rides with the reason. Without it the
                // author rewrites the whole script from a blank page to fix
                // whatever the reason names — observed live: a missing
                // one-line `export const meta` marker cost a full 85-minute
                // regeneration, because the previous 35,000 characters were
                // never handed back.
                //
                // The retry is also told not to repeat the investigation. The
                // brief it shares with the first attempt orders a full read of
                // the PRD and every task file, and the first attempt spent 27
                // file reads on exactly that — almost all of its 85 minutes,
                // each read re-sending the whole grown context. Handing the
                // draft back saves the writing; this saves the reading, which
                // was the larger half.
                &retry_feedback
                    .map(|reason| match rejected_draft {
                        Some(draft) => [
                            &format!("YOUR PREVIOUS ATTEMPT WAS REJECTED: {reason}. Fix EVERY defect listed.\n\n"),
                            "Your previous draft follows in full. REPAIR IT — keep everything that was already correct and change only what the rejection names. Return the complete repaired script, not a diff.\n\n",
                            "DO NOT REPEAT THE INVESTIGATION. The draft below already reflects the task files and repository state you read last time; re-reading them all costs far more than the repair does. Re-read ONLY the specific files the rejection above forces you to check, and none of the others.\n\n",
                            &format!("--- BEGIN REJECTED DRAFT ---\n{draft}\n--- END REJECTED DRAFT ---\n"),
                        ]
                        .concat(),
                        None => {
                            format!("YOUR PREVIOUS ATTEMPT WAS REJECTED: {reason}. Fix EVERY defect listed.\n")
                        }
                    })
                    .unwrap_or_default(),
            ),
            // The closed half of the learning loop. Prior runs' forensic
            // records were injected here once and cost 340KB of context —
            // a hundred stage ids and artifact paths, not one sentence saying
            // what to do differently. What goes in now is the curated
            // distillation: fixed prose selected by rule, merged across runs,
            // carrying counts and no identifiers, capped in both count and
            // bytes. The in-flight run is excluded so a resume is never taught
            // by its own partial record.
            (
                "curated_lessons",
                &archon_workflow::curated_lessons_block(
                    &self.workflow_store,
                    Some(self.run_id.as_str()),
                ),
            ),
            (
                "reference",
                &archon_workflow::v2::script::render_dialect_reference(
                    self.task_universe.as_ref(),
                ),
            ),
        ]);
        bootstrap.script_args = Some(serde_json::json!({ "author_task": author_task }));
        let summary = bootstrap.run(V3_AUTHOR_BOOTSTRAP).await?;
        let raw = summary.script_result.ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "workflow author bootstrap produced no script result".to_string(),
            )
        })?;
        let value: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
            WorkflowError::SpecInvalid(format!(
                "workflow author bootstrap result was not JSON: {err}"
            ))
        })?;
        let source = value
            .get("workflow_js")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "workflow author did not return a usable script: {value}"
                ))
            })?;
        validate_authored_workflow_source(source)
    }
}

#[cfg(test)]
#[path = "workflow_live_v3_author_tests.rs"]
mod workflow_live_v3_author_tests;
