const V3_AUTHOR_BOOTSTRAP: &str = r#"
async function workflow(w) {
  const authored = await w.agent(
    "author-workflow-script",
    { tier: "planner", task: args.author_task }
  );
  const source =
    (authored && typeof authored.workflow_js === "string" && authored.workflow_js) ||
    (authored && authored.data && typeof authored.data.workflow_js === "string" && authored.data.workflow_js) ||
    (authored && authored.result && authored.result.data &&
      typeof authored.result.data.workflow_js === "string" && authored.result.data.workflow_js) ||
    null;
  if (typeof source !== "string" || source.trim().length < 80) {
    return { authoring_failed: true, summary: authored && authored.summary };
  }
  return { workflow_js: source };
}
"#;

impl WorkflowV2ScriptRunner {
    /// v3 entry: author workflow.js if absent (journaled, cache-keyed on the composed brief
    /// (task paths + per-file content fingerprints + lessons)), persist it, then execute it. Re-runs with an unchanged
    /// authored script replay unchanged call prefixes from the store.
    pub(super) async fn run_authored_script_lifecycle(
        self,
        authored_path: std::path::PathBuf,
        governed_learning_context: serde_json::Value,
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
            let first = match self
                .author_workflow_source(None, &governed_learning_context)
                .await
            {
                Ok(source) => match validate_authored_plan(&source, &expected_task_ids).await {
                    Ok(()) => Ok(source),
                    Err(reason) => Err(reason),
                },
                Err(err) => Err(format!(
                    "the authoring envelope was unusable ({err}); the complete script text must be the data.workflow_js field of the standard result envelope"
                )),
            };
            let source = match first {
                Ok(source) => source,
                Err(reason) => {
                    let source = self
                        .author_workflow_source(Some(&reason), &governed_learning_context)
                        .await?;
                    if let Err(reason) = validate_authored_plan(&source, &expected_task_ids).await {
                        return Err(WorkflowError::SpecInvalid(format!(
                            "authored workflow failed its dry-run pre-flight twice; last error: {reason}"
                        )));
                    }
                    source
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

    async fn author_workflow_source(
        &self,
        retry_feedback: Option<&str>,
        governed_learning_context: &serde_json::Value,
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
                self.runtime.target_repository_root.as_deref().unwrap_or("<none>"),
            ),
            ("source_roots", &source_roots),
            ("task_paths", &task_paths),
            (
                "retry_feedback",
                &retry_feedback
                    .map(|reason| format!(
                        "YOUR PREVIOUS ATTEMPT WAS REJECTED: {reason}. Fix EVERY defect listed.\n"
                    ))
                    .unwrap_or_default(),
            ),
            ("learning_context", &governed_learning_context.to_string()),
            ("reference", V3_PRIMITIVE_REFERENCE),
        ]);
        bootstrap.script_args = Some(serde_json::json!({ "author_task": author_task }));
        let summary = bootstrap
            .run_without_terminal_status(V3_AUTHOR_BOOTSTRAP)
            .await?;
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

fn validate_authored_workflow_source(source: &str) -> archon_workflow::WorkflowResult<String> {
    let source = source.trim();
    if source.len() < 80 {
        return Err(WorkflowError::SpecInvalid(
            "authored workflow.js is shorter than the minimum usable source length".to_string(),
        ));
    }
    if workflow_meta_marker_offset(source).is_none() {
        return Err(WorkflowError::SpecInvalid(
            "authored workflow.js is missing the required `export const meta` declaration"
                .to_string(),
        ));
    }
    Ok(source.to_string())
}

fn validate_authored_task_accounting(
    script_result: Option<&str>,
    expected: &std::collections::BTreeSet<String>,
) -> archon_workflow::WorkflowResult<()> {
    if expected.is_empty() {
        return Ok(());
    }
    let raw = script_result.ok_or_else(|| {
        WorkflowError::SpecInvalid("authored workflow returned no task accounting".to_string())
    })?;
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|err| {
        WorkflowError::SpecInvalid(format!(
            "authored workflow task accounting was not JSON: {err}"
        ))
    })?;
    let accepted = value
        .get("accepted")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "authored workflow task accounting omitted `accepted`".to_string(),
            )
        })?;
    let blocked = value
        .get("blocked")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "authored workflow task accounting omitted `blocked`".to_string(),
            )
        })?;
    // The adversarial review and source-coverage audit are mandatory: their
    // output arrays must be present (possibly empty) — a run that never ran
    // them cannot produce honest completeness claims.
    for field in MANDATED_RESULT_FIELDS {
        if value
            .get(field)
            .and_then(serde_json::Value::as_array)
            .is_none()
        {
            return Err(WorkflowError::SpecInvalid(format!(
                "authored workflow accounting omitted `{field}` — the adversarial review and source-coverage audit agents are mandatory"
            )));
        }
    }
    let mut accounted = std::collections::BTreeSet::new();
    for task_id in accepted {
        let task_id = task_id.as_str().ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "authored workflow `accepted` entries must be task ids".to_string(),
            )
        })?;
        if !accounted.insert(task_id.to_string()) {
            return Err(WorkflowError::SpecInvalid(format!(
                "authored workflow accounted for task `{task_id}` more than once"
            )));
        }
    }
    for entry in blocked {
        let task_id = entry
            .get("taskId")
            .or_else(|| entry.get("task_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(
                    "authored workflow blocked entries must name a taskId".to_string(),
                )
            })?;
        entry
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "authored workflow blocked task `{task_id}` without evidence"
                ))
            })?;
        if !accounted.insert(task_id.to_string()) {
            return Err(WorkflowError::SpecInvalid(format!(
                "authored workflow accounted for task `{task_id}` more than once"
            )));
        }
    }
    let unknown = accounted.difference(expected).cloned().collect::<Vec<_>>();
    let missing = expected.difference(&accounted).cloned().collect::<Vec<_>>();
    if !unknown.is_empty() || !missing.is_empty() {
        return Err(WorkflowError::SpecInvalid(format!(
            "authored workflow task accounting diverged: missing={missing:?} unknown={unknown:?}"
        )));
    }
    Ok(())
}
