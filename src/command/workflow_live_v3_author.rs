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

const V3_AUTHOR_TASK: &str = r#"Author a complete workflow.js orchestration script for the provided decomposed task universe, following the dialect reference exactly. The script must cover EVERY task in the universe: implement, verify, and honestly account for each one. The input includes governed learning context from previous runs — apply its lessons: avoid the recorded failure classes, keep whatever prevented false completions, and steer agents away from repair patterns that previously churned. Reply with a JSON object whose data.workflow_js field contains ONLY the complete script text (no fences, no commentary)."#;

/// Bootstrap script (legacy dialect): one journaled reduce call authors the
/// workflow source and returns it through the script result channel.
const V3_AUTHOR_BOOTSTRAP: &str = r#"
async function workflow(w) {
  const authored = await w.reduce(
    "author-workflow-script",
    [args.task_universe, args.primitive_reference, args.retry_feedback || null, args.learning_context || null],
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
        governed_learning_context: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let authored_source = if authored_path.exists() {
            let source = std::fs::read_to_string(&authored_path).map_err(|err| WorkflowError::Io {
                path: authored_path.clone(),
                source: err,
            })?;
            let source = validate_authored_workflow_source(&source)?;
            // Pre-flight: the persisted script must still plan real work.
            if let Err(reason) = validate_authored_plan(&source).await {
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
            let mut source = self
                .author_workflow_source(None, &governed_learning_context)
                .await?;
            if let Err(reason) = validate_authored_plan(&source).await {
                source = self
                    .author_workflow_source(Some(&reason), &governed_learning_context)
                    .await?;
                if let Err(reason) = validate_authored_plan(&source).await {
                    return Err(WorkflowError::SpecInvalid(format!(
                        "authored workflow failed its dry-run pre-flight twice; last error: {reason}"
                    )));
                }
            }
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
        bootstrap.script_args = Some(serde_json::json!({
            "task_universe": self.task_universe,
            "primitive_reference": V3_PRIMITIVE_REFERENCE,
            "author_task": V3_AUTHOR_TASK,
            "learning_context": governed_learning_context,
            "retry_feedback": retry_feedback.map(|reason| format!(
                "Your previous script was REJECTED before execution: {reason}. Fix EVERY defect listed and return the corrected complete script."
            )),
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
                    "workflow author did not return a usable script: {value}"
                ))
            })?;
        validate_authored_workflow_source(source)
    }
}

/// Dry-run pre-flight: execute the authored script against the recording stub
/// host and require it to PLAN real work. A script that would spawn zero
/// agents must never reach live execution.
async fn validate_authored_plan(source: &str) -> Result<(), String> {
    let planned = dry_run_workflow_plan(source, None)
        .await
        .map_err(|err| format!("dry run failed: {err}"))?;
    let work_calls = planned
        .iter()
        .filter(|call| {
            matches!(
                call.method,
                WorkflowV2HostMethod::Agent
                    | WorkflowV2HostMethod::Implementation
                    | WorkflowV2HostMethod::Fanout
                    | WorkflowV2HostMethod::Parallel
            )
        })
        .count();
    if work_calls == 0 {
        return Err(format!(
            "the script plans ZERO agent calls across {} host call(s) — every implement/verify body must actually invoke agent() and be awaited",
            planned.len()
        ));
    }
    validate_mandatory_review_calls(&planned)?;
    Ok(())
}

/// Single source of truth for the mandated post-work reviews. The reference
/// text, the validator, the accounting fields, and the tests all derive from
/// these constants — a drift-guard unit test pins the reference against them.
pub(super) const MANDATED_REVIEWS: [(&str, &str, bool); 2] = [
    ("adversarial-review", "adversarial review", true),
    ("coverage-audit", "source-coverage audit", false),
];
pub(super) const MANDATED_RESULT_FIELDS: [&str; 2] =
    ["adversarial_findings", "uncovered_requirements"];
const CRITIC_TIER: &str = "critic";

pub(super) fn mandate_call_hint(label: &str, requires_critic: bool) -> String {
    if requires_critic {
        format!("await agent(..., {{ label: '{label}', tier: '{CRITIC_TIER}' }})")
    } else {
        format!("await agent(..., {{ label: '{label}' }})")
    }
}

/// The prelude mints agent ids as `<label>-<ordinal>`; a mandate matches only
/// that exact shape (or the bare label), never labels that merely extend it.
fn call_id_matches_label(id: &str, label: &str) -> bool {
    if id == label {
        return true;
    }
    id.strip_prefix(label)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|ordinal| !ordinal.is_empty() && ordinal.bytes().all(|b| b.is_ascii_digit()))
}

fn is_mandated_review_call(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Agent
        && MANDATED_REVIEWS
            .iter()
            .any(|(label, _, _)| call_id_matches_label(&call.id, label))
}

fn is_task_work_call(call: &WorkflowV2HostCall) -> bool {
    !is_mandated_review_call(call)
        && matches!(
            call.method,
            WorkflowV2HostMethod::Agent
                | WorkflowV2HostMethod::Fanout
                | WorkflowV2HostMethod::Implementation
                | WorkflowV2HostMethod::Parallel
        )
}

/// Enforce the mandated reviews on the EXECUTED/PLANNED call sequence:
/// present, as separate top-level read-only agent() calls, with the critic
/// tier where required, positioned AFTER all task work. Reports EVERY defect
/// in one error and names the ACTUAL cause for near-misses.
fn validate_mandatory_review_calls(planned: &[WorkflowV2HostCall]) -> Result<(), String> {
    let last_work = planned.iter().rposition(is_task_work_call);
    let mut defects: Vec<String> = Vec::new();
    for (label, purpose, requires_critic) in MANDATED_REVIEWS {
        let hint = mandate_call_hint(label, requires_critic);
        let matched = planned.iter().enumerate().find(|(_, call)| {
            call.method == WorkflowV2HostMethod::Agent && call_id_matches_label(&call.id, label)
        });
        let Some((index, call)) = matched else {
            // Near-miss diagnosis: the label exists but under the wrong call
            // kind (agents() batch → Parallel, write:true → Fanout), or only
            // as an extended label — name the real defect, not "omitted".
            if let Some(wrong_kind) = planned.iter().find(|call| {
                call.method != WorkflowV2HostMethod::Agent
                    && call_id_matches_label(&call.id, label)
            }) {
                defects.push(format!(
                    "the {purpose} labeled `{label}` ran as w.{} — the mandated reviews must be SEPARATE top-level read-only agent() calls, never inside agents() batches and never with write:true; use: {hint}",
                    wrong_kind.method.as_str()
                ));
            } else if let Some(extended) = planned.iter().find(|call| {
                call.method == WorkflowV2HostMethod::Agent
                    && call.id.starts_with(label)
                    && !call_id_matches_label(&call.id, label)
            }) {
                defects.push(format!(
                    "no {purpose} agent with the exact label `{label}` (found `{}` — extended labels do not count); use: {hint}",
                    extended.id
                ));
            } else {
                defects.push(format!(
                    "the mandatory {purpose} agent with exact label `{label}` is missing; add: {hint}"
                ));
            }
            continue;
        };
        if requires_critic
            && !call
                .options
                .role
                .as_deref()
                .is_some_and(|role| role.eq_ignore_ascii_case(CRITIC_TIER))
        {
            defects.push(format!(
                "the {purpose} agent `{}` must use tier '{CRITIC_TIER}' so it routes to the dedicated adversarial reviewer; use: {hint}",
                call.id
            ));
        }
        if let Some(last_work) = last_work
            && index < last_work
        {
            defects.push(format!(
                "the {purpose} agent `{}` runs BEFORE task work finishes — both mandated reviews must come after ALL agent, implementation, fanout, and parallel calls",
                call.id
            ));
        }
    }
    if defects.is_empty() {
        return Ok(());
    }
    Err(format!(
        "mandated-review defects (fix EVERY one): {}",
        defects.join("; AND ")
    ))
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
