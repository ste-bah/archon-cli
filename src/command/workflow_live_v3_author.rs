// v3 script lifecycle: a planner agent AUTHORS workflow.js from the task
// universe using the documented primitive dialect, then the authored script
// executes through the QuickJS runtime — composition is code, judgment is a
// script-spawned agent, and every write flows through the same gauntlet.

/// Reference handed to the author agent. Generic by construction: it
/// documents the dialect, never the fixture domain.
const V3_PRIMITIVE_REFERENCE: &str = r#"WORKFLOW SCRIPT DIALECT (v3)

Shape — top-level script, exactly like this (no wrapper function):

  export const meta = { name: '<kebab-name>', description: '<one line>', phases: [{ title, detail }] }

  phase('First Phase')
  const first = await agent('...prompt...', { label: 'first-step' })
  log('first step done')

  phase('Second Phase')
  const second = await agent(`...uses ${first.summary} verbatim...`, { label: 'second-step' })

  phase('Review')
  const adversarial = await agent('Try to FALSIFY every accepted claim above using the actual files and tests: ...', { label: 'adversarial-review', tier: 'critic' })
  const coverage = await agent('Compare the source requirements document against the task list; name every requirement no task covers: ...', { label: 'coverage-audit' })

  return {
    accepted: acceptedTaskIds,
    blocked: blockedTasks,
    adversarial_findings: findingsFrom(adversarial),
    uncovered_requirements: gapsFrom(coverage),
    notes: 'short honest summary',
  }
  // (acceptedTaskIds/blockedTasks are arrays you build during the run;
  //  findingsFrom/gapsFrom are your own small helpers reading the envelopes.)

Statements run at the top level: bare phase()/log() (no await needed), `await agent(...)`, and a final top-level `return`.

Primitives:
- await agent(prompt, opts) -> result envelope { status, summary, data, result }  // MUST be awaited
  opts: {
    label: '<short-kebab-label>'          // required; call ids derive from it deterministically
    write: true,                          // spawn a WRITE agent in a sealed worktree through the write gauntlet
    taskIds: ['<canonical task id>'],     // required when write:true
    targetFiles: ['path/one.ext'],        // LITERAL repo-relative file paths ONLY (never descriptions); the write agent owns exactly these
    focusedTests: ['exact test command'], // write:true — commands proving the change; must match >0 tests
    artifacts: ['relative/artifact.path'],// artifacts the work must produce
    tier: 'coder' | 'reducer' | 'analysis' | 'critic'   // 'critic' routes to the dedicated adversarial reviewer
  }
  Without write:true the agent is read-only (verification, judgment, exploration).
  AGENT SELECTION IS AUTOMATIC: the host picks the best registry agent from the stage type, tier, and prompt
  content (e.g. systems-language implementation routes to the systems-coder specialist; tier 'critic' routes to
  the adversarial reviewer). Describe the WORK precisely; do not invent agent names. To pin a specific registry
  agent deliberately, pass its exact name as `tier`.
- await agents([{ prompt, label, taskIds, targetFiles, focusedTests, artifacts }, ...], opts) -> batch envelope
  Runs INDEPENDENT specs concurrently through ONE host call. opts: { write: true for write agents, tier, maxParallelism, task }.
  maxParallelism is a HINT: the host caps it at the configured agent limit and queues the rest. Use for tasks with no
  dependency between them and no shared target files; per-item outcomes are in the returned envelope.
- phase('Title')                                // progress + journal marker; no await needed
- phase('Title', async () => { ... })           // marker then runs the body — MUST be awaited; returns the body's result
- log('message')                                // journal note; no await needed
- await pipeline(items, [async stage(item) => next, ...]) -> results  // same stages over each item, sequentially

Rules the script must follow:
- AWAIT EVERY agent(), agents(), pipeline(), and phase-with-body call. Never fire-and-forget real work: a workflow that returns while work is pending FAILS the run with a dropped-call error. (Bare phase()/log() markers are the only calls that need no await.)
- SEQUENTIAL vs PARALLEL: tasks connected by dependency_ids or touching the same files run SEQUENTIALLY (await one before starting the next). INDEPENDENT tasks may run concurrently — prefer `await agents([...])` (one host call, host-managed safe parallelism); `await Promise.all([...agent()...])` is acceptable for independent READ-ONLY work only. Never parallelize write work outside agents(). EXCEPTION: the two mandated reviews below must be SEPARATE top-level `await agent(...)` calls — never inside agents() batches, never write:true, never conditional — placed AFTER all task work.
- Work tasks in dependency order (each task in the universe lists dependency_ids).
- For each task: implement with a write agent, then verify with a read-only agent whose prompt names EXACT, module-qualified test commands; a test filter matching zero tests is never evidence.
- Read every returned envelope. If status is not accepted/noop, the envelope carries the verbatim gate error: retry with SPECIFIC corrected instructions (exact command, exact path), at most 3 retries per task, then record the task as blocked with the evidence.
- Never edit an existing artifact instance to satisfy a check; produce new artifacts through the real pipeline.
- An honest block naming a real gap is success; fabricated acceptance is failure.
- Deterministic code only (no Math.random, no Date.now); pass any needed timestamps via prompts.
- MANDATORY after all task work, before returning:
  1. An ADVERSARIAL REVIEW agent: `await agent(<claims + evidence>, { label: 'adversarial-review', tier: 'critic' })` — tier 'critic' routes to the dedicated adversarial reviewer (the sherlock agent) when the project defines one. Instruct it to try to FALSIFY every accepted task's claims — probe actual files, run focused checks, hunt for hollow artifacts, untested paths, and claims that outrun evidence. Its findings go into the return value verbatim.
  2. A SOURCE-COVERAGE AUDIT agent (read-only, label 'coverage-audit'): give it the source requirements document path and the full task list and instruct it to name every normative requirement that NO task in the universe covers. Report gaps honestly; do NOT implement work outside the given tasks.
- Return {
    accepted: [...taskIds],
    blocked: [{ taskId, reason }],
    adversarial_findings: [ '<finding or empty>' ],
    uncovered_requirements: [ '<requirement no task covers, or empty>' ],
    notes: '<short honest summary>'
  } accounting for EVERY task id exactly once across accepted+blocked; adversarial_findings and uncovered_requirements MUST come from the two mandatory agents, never invented or omitted."#;

const V3_AUTHOR_TASK_TEMPLATE: &str = r#"Author the complete workflow.js orchestration script for this decomposed task set. INVESTIGATE BEFORE WRITING — you have READ tools (Read, Grep, Glob); you have NO shell and must NOT run commands or create/modify ANY files. Your ONLY deliverable is the result envelope.

Required investigation (do it; cite the files you actually read in evidence):
1. READ the source requirements document(s) under the source roots below, and EVERY task file listed.
2. Inspect the repository tree with Glob/Read (key directories, the files each task declares); distrust any existing status/acceptance documents — verify against the live tree.
3. For each task, extract its EXACT declared target files, dependencies, acceptance criteria, and artifact contracts — honor them verbatim, never invent paths. Use canonical task ids verbatim in taskIds.
4. Decide sequential vs parallel FROM THE TASK DATA: tasks editing shared files or linked by dependencies run sequentially; only genuinely independent tasks may batch.

Then write the script per the dialect reference and SELF-CHECK before returning:
- every canonical task id appears in EXACTLY ONE write agent() call's taskIds with that task's declared target files (never one umbrella call claiming many tasks);
- a task that is already implemented still gets its write agent — instruct that agent to return the typed no-op (status noop, idempotent_noop true, task_coverage evidence) when it verifies nothing needs changing; NEVER make cosmetic edits just to show work;
- every write call has focused test commands; the two mandated reviews are present, exactly labeled, after all work;
- meta.phases matches the phase() calls; the accounting return covers every task id exactly once;
- the script text must not contain confirmation questions or the phrases "restored context"/"previous session summary".

Reply with the standard JSON result envelope; put ONLY the complete script text in data.workflow_js (no fences) — workflow_js must sit INSIDE data. Include evidence entries naming the files you read.

Repository root: {repo_root}
Source requirement roots: {source_roots}
Task files (read every one; the fingerprint changes when the file changes):
{task_paths}

{retry_feedback}
Governed learning context from previous runs (apply its lessons):
{learning_context}

DIALECT REFERENCE:
{reference}"#;

/// Single-pass placeholder substitution: each `{name}` is looked up once —
/// substituted content is never re-scanned, so run-derived text (learning
/// context, retry errors) cannot inject other placeholders.
fn compose_author_brief(values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(V3_AUTHOR_TASK_TEMPLATE.len());
    let mut rest = V3_AUTHOR_TASK_TEMPLATE;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let name = &rest[start + 1..start + len];
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            out.push_str(&rest[..start]);
            out.push_str(value);
            rest = &rest[start + len + 1..];
        } else {
            out.push_str(&rest[..start + 1]);
            rest = &rest[start + 1..];
        }
    }
    out.push_str(rest);
    debug_assert!(
        !["{repo_root}", "{source_roots}", "{task_paths}", "{retry_feedback}", "{learning_context}", "{reference}"]
            .iter()
            .any(|token| out.contains(token)),
        "author brief has unsubstituted placeholders"
    );
    out
}

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
            let source = std::fs::read_to_string(&authored_path).map_err(|err| WorkflowError::Io {
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
            let first = match self.author_workflow_source(None, &governed_learning_context).await {
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
        let summary = self.run(&authored_source).await?;
        validate_mandatory_review_calls(&summary.calls).map_err(|reason| {
            WorkflowError::SpecInvalid(format!(
                "the executed run violated the mandated-review contract ({reason}); the live call sequence diverged from the pre-flight plan (likely conditional review calls) — delete {} to re-author with unconditional reviews",
                authored_path.display()
            ))
        })?;
        validate_authored_task_accounting(summary.script_result.as_deref(), &expected_task_ids)?;
        Ok(summary)
    }

    async fn author_workflow_source(
        &self,
        retry_feedback: Option<&str>,
        governed_learning_context: &serde_json::Value,
    ) -> archon_workflow::WorkflowResult<String> {
        let mut bootstrap = self.clone();
        // Authoring must never adopt a cached record from a prior session:
        // frontier reuse ignores the input hash, which would replay a stale
        // script for BOTH bounded attempts (retry feedback unseen).
        bootstrap.adopt_accepted_cache = false;
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
    // The adversarial review and source-coverage audit are mandatory: their
    // output arrays must be present (possibly empty) — a run that never ran
    // them cannot produce honest completeness claims.
    for field in MANDATED_RESULT_FIELDS {
        if value.get(field).and_then(serde_json::Value::as_array).is_none() {
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
