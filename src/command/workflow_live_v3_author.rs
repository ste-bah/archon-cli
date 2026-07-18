// v3 script lifecycle: a planner agent AUTHORS workflow.js from the task
// universe using the documented primitive dialect, then the authored script
// executes through the QuickJS runtime — composition is code, judgment is a
// script-spawned agent, and every write flows through the same gauntlet.

/// Reference handed to the author agent. Generic by construction: it
/// documents the dialect, never the fixture domain.
const V3_PRIMITIVE_REFERENCE: &str = r#"WORKFLOW SCRIPT DIALECT (v3)

Shape (both exports are required):
  export const meta = { name: '<kebab-name>', description: '<one line>', phases: [{ title, detail }] }
  export default async function workflow({ agent, phase, log, pipeline, w }) { ... return <result object>; }

Primitives:
- await agent(prompt, opts) -> result envelope { status, summary, data, result }
  opts: {
    label: '<short-kebab-label>'          // required; call ids derive from it deterministically
    write: true,                          // spawn a WRITE agent in a sealed worktree through the write gauntlet
    taskIds: ['<canonical task id>'],     // required when write:true
    targetFiles: ['path/one.ext'],        // files the write agent owns
    focusedTests: ['exact test command'], // write:true — commands proving the change; must match >0 tests
    artifacts: ['relative/artifact.path'],// artifacts the work must produce
    tier: 'coder' | 'reducer' | 'analysis'
  }
  Without write:true the agent is read-only (verification, judgment, exploration).
- await phase('Title')   // progress + journal marker
- await log('message')   // journal note
- await pipeline(items, [async stage(item) => next, ...]) -> results  // same stages over each item, sequentially

Rules the script must follow:
- Work tasks in dependency order (each task in the universe lists dependency_ids).
- For each task: implement with a write agent, then verify with a read-only agent whose prompt names EXACT, module-qualified test commands; a test filter matching zero tests is never evidence.
- Read every returned envelope. If status is not accepted/noop, the envelope carries the verbatim gate error: retry with SPECIFIC corrected instructions (exact command, exact path), at most 3 retries per task, then record the task as blocked with the evidence.
- Never edit an existing artifact instance to satisfy a check; produce new artifacts through the real pipeline.
- An honest block naming a real gap is success; fabricated acceptance is failure.
- Deterministic code only (no Math.random, no Date.now); pass any needed timestamps via prompts.
- Return { accepted: [...taskIds], blocked: [{ taskId, reason }], notes: '<short honest summary>' }."#;

const V3_AUTHOR_TASK: &str = r#"Author a complete workflow.js orchestration script for the provided decomposed task universe, following the dialect reference exactly. The script must cover EVERY task in the universe: implement, verify, and honestly account for each one. Reply with a JSON object whose data.workflow_js field contains ONLY the complete script text (no fences, no commentary)."#;

/// Bootstrap script (legacy dialect): one journaled reduce call authors the
/// workflow source and returns it through the script result channel.
const V3_AUTHOR_BOOTSTRAP: &str = r#"
async function workflow(w) {
  const authored = await w.reduce(
    "author-workflow-script",
    [args.task_universe, args.primitive_reference],
    { tier: "reducer", task: args.author_task }
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
    /// v3 entry: author workflow.js if absent (journaled, cache-keyed on the
    /// universe), persist it, then execute it. Re-runs with an unchanged
    /// authored script replay unchanged call prefixes from the store.
    pub(super) async fn run_authored_script_lifecycle(
        self,
        authored_path: std::path::PathBuf,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let authored_source = if authored_path.exists() {
            let source = std::fs::read_to_string(&authored_path).map_err(|err| WorkflowError::Io {
                path: authored_path.clone(),
                source: err,
            })?;
            validate_authored_workflow_source(&source)?
        } else {
            let mut bootstrap = self.clone();
            bootstrap.script_args = Some(serde_json::json!({
                "task_universe": self.task_universe,
                "primitive_reference": V3_PRIMITIVE_REFERENCE,
                "author_task": V3_AUTHOR_TASK,
            }));
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
                        "workflow author did not return a usable script: {}",
                        value
                    ))
                })?;
            let source = validate_authored_workflow_source(source)?;
            std::fs::write(&authored_path, &source).map_err(|err| WorkflowError::Io {
                path: authored_path.clone(),
                source: err,
            })?;
            source
        };
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
        let summary = self.run(&authored_source).await?;
        validate_authored_task_accounting(summary.script_result.as_deref(), &expected_task_ids)?;
        Ok(summary)
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
        WorkflowError::SpecInvalid(format!("authored workflow task accounting was not JSON: {err}"))
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
