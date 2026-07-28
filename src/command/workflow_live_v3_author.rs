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

  function remediationEvidence(env) {
    if (!env) return 'no envelope'
    let outputSummaryBudget = 4000
    return JSON.stringify(env, (key, value) => {
      if (key !== 'output_summary' || typeof value !== 'string') return value
      const kept = value.slice(0, outputSummaryBudget)
      outputSummaryBudget -= kept.length
      return kept.length === value.length ? kept : `${kept}\n[output_summary truncated]`
    })
  }

  phase('Task Work')
  // Author ONE implement -> verify -> REMEDIATE block for EVERY canonical task in
  // the universe by ITERATING THE FULL TASK LIST. This loop is mandatory: do NOT
  // implement a single task and stop, and do NOT re-run the same task id — every
  // task from the first to the last must get its own block with its own labels.
  // Enumerate the ACTUAL task ids and their real task-file paths / target files
  // here — one entry per canonical task in the universe, in dependency order.
  const tasks = [
    { id: 'TASK-X-001', file: '<task file path for TASK-X-001>', targetFiles: ['src/module.ext'] },
    { id: 'TASK-X-002', file: '<task file path for TASK-X-002>', targetFiles: ['src/other.ext'] },
    // ...one entry for EVERY remaining canonical task id in the universe...
  ]
  const acceptedTaskIds = []
  const blockedTasks = []
  for (const t of tasks) {
    // A verifier that demotes the task is not the end: feed its verbatim findings
    // to a fresh write agent and re-verify, up to 3 attempts, then record blocked.
    let impl = await agent(`Implement ${t.id} per ${t.file}. Repository root: <repo root>. Re-inspect the current state FIRST — if the work is genuinely already done, return the typed no-op. Prove your change with tests you run yourself.`, { label: `implement-${t.id.toLowerCase()}`, write: true, taskIds: [t.id], targetFiles: t.targetFiles })
    let check = await agent(`You did NOT implement ${t.id} — be suspicious of its self-report. Re-read ${t.file}, inspect the actual code, and run whatever tests YOU judge prove or disprove the acceptance criteria.`, { label: `verify-${t.id.toLowerCase()}`, verify: true, taskIds: [t.id] })
    // Budget follows PROGRESS, not a flat count: it extends past the base
    // attempts only while the FIRST verifier's gap set is still shrinking, and
    // stops on a plateau. Do not replace this with a fixed bound.
    const budget = remediationBudget()
    for (let attempt = 2; budget.shouldContinue(attempt - 1, check) && (!isAccepted(impl) || !isAccepted(check)); attempt += 1) {
      const rejectedAttempt = `Implementation envelope:\n${remediationEvidence(impl)}\nVerifier envelope:\n${remediationEvidence(check)}`
      impl = await agent(`Remediate ${t.id}. The previous attempt was REJECTED. Fix exactly what these verbatim implementation and verifier envelopes name; do not re-argue them:\n${rejectedAttempt}\nOriginal goal: implement ${t.id} per ${t.file}. Repository root: <repo root>. Prove the fix with tests you run yourself.`, { label: `remediate-${t.id.toLowerCase()}-${attempt}`, write: true, taskIds: [t.id], targetFiles: t.targetFiles })
      check = await agent(`You did NOT implement ${t.id} — be suspicious. The previous attempt was rejected with these verbatim findings:\n${rejectedAttempt}\nRe-read ${t.file}, inspect the actual code, and run whatever tests YOU judge prove or disprove the acceptance criteria.`, { label: `verify-${t.id.toLowerCase()}-${attempt}`, verify: true, taskIds: [t.id] })
    }
    isAccepted(impl) && isAccepted(check) ? acceptedTaskIds.push(t.id) : blockedTasks.push({ taskId: t.id, reason: summarize(check) })
  }

  phase('Review')
  // The runtime PROVIDES the two mandatory reviews as built-in primitives:
  // adversarialReview and coverageAudit each fan out ONE critic reviewer per
  // accepted task (bounded, so a large deliverable never overflows one context)
  // then reduce over the findings. You do NOT author the map/reduce shape — just
  // pass the accepted task ids and a bounded-evidence function. Each returns the
  // final findings array for the accounting below.
  const adversarial_findings = await adversarialReview(acceptedTaskIds, { evidenceFor: boundedEvidenceFor })
  const uncovered_requirements = await coverageAudit(acceptedTaskIds, { evidenceFor: boundedEvidenceFor })
  // Reviews find problems AFTER every task is accepted, so nothing downstream
  // would ever act on them. remediateFindings runs one bounded fix+re-verify
  // pass over the findings that name a task and returns what is still open —
  // report review_remediation in the accounting so unresolved findings and
  // findings naming no task stay visible instead of being quietly dropped.
  // Pass blockedTasks too: the reviews only inspect ACCEPTED tasks, so a task
  // that exhausted its own budget can never appear in their findings and would
  // otherwise be reported and abandoned. It gets one more bounded attempt here.
  const review_remediation = await remediateFindings([...adversarial_findings, ...uncovered_requirements], { blockedTasks, taskFileFor: (id) => (tasks.find((t) => t.id === id) || {}).file, targetFilesFor: (id) => (tasks.find((t) => t.id === id) || {}).targetFiles })

  return {
    accepted: acceptedTaskIds,
    blocked: blockedTasks,
    adversarial_findings,
    uncovered_requirements,
    review_remediation,
    notes: 'short honest summary',
  }
  // Your own small helpers, defined at the top of the script:
  //   isAccepted(env) -> env && (env.status === 'accepted' || env.status === 'noop')
  //   remediationEvidence(env) -> JSON.stringify the complete envelope with every
  //      finding intact, but share a 4,000-character budget across only its
  //      commands_run[*].output_summary strings and mark any truncation
  //   summarize(env)  -> short text used only for final blocked accounting
  //   boundedEvidenceFor(taskId) -> a compact, bounded evidence array for a task
  //      id (its accepted claims/artifacts) — the reviewers falsify against it

Statements run at the top level: bare phase()/log() (no await needed), `await agent(...)`, and a final top-level `return`.

Primitives:
- await agent(prompt, opts) -> result envelope { status, summary, data, result }  // MUST be awaited
  opts: {
    label: '<short-kebab-label>'          // required; call ids derive from it deterministically
    write: true,                          // spawn a WRITE agent in a sealed worktree through the write gauntlet
    taskIds: ['<canonical task id>'],     // required when write:true
    targetFiles: ['path/one.ext'],        // LITERAL repo-relative file paths ONLY (never descriptions); the write agent owns exactly these
    verify: true,                         // REQUIRED on per-task verifiers: routes the agent through the host
                                          // verification machinery WITH command execution. Without it (or a
                                          // non-empty focusedTests) the agent has NO shell and any test runs
                                          // it claims are downgraded to inspection — hollow verification.
    focusedTests: ['test command'],       // OPTIONAL — only commands you VERIFIED exist (a wrong package or
                                          // module name fails the run); omit to let the agent choose its own.
                                          // If given, must match >0 tests. On a read-only agent a non-empty
                                          // list routes through the verification machinery like verify:true.
    artifacts: ['relative/artifact.path'],// artifacts the work must produce
    tier: 'coder' | 'reducer' | 'analysis' | 'critic'   // 'critic' routes to the dedicated adversarial reviewer
  }
  Without write:true the agent is read-only (verification, judgment, exploration). Per-task verification
  agents MUST set `verify: true` (or focusedTests): they then run through the host verification machinery
  and can EXECUTE their test commands, with zero-match protection attached. The adversarial reviewer is
  read-only by design — give it the verifier outputs and file paths; it falsifies by inspection.
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
- SEQUENTIAL by default, in dependency order. Only genuinely independent tasks (no shared files, no dependency link) may run concurrently via `await agents([...])`; parallel writes outside agents() are forbidden.
- PER TASK, TWO STAGES, GOAL-ORIENTED PROMPTS — agents are capable sessions with their own tools; give them goals and context, never command scripts to obey:
  1. IMPLEMENT (write agent): give it the task file PATH, the repository root, and the goal; for artifact work tell it to use `project_artifact_root` from its OWN stage input (the host stamps it there — never guess or invent an artifact path yourself). Tell it to READ the task file and RE-INSPECT the current repo/artifact state FIRST — if the work is genuinely already done it returns the typed no-op (status noop, idempotent_noop true, task_coverage evidence) instead of redoing or cosmetically editing anything; the workflow must be safe to re-run. It decides how to implement and how to prove it, runs its own tests, and fixes its own command mistakes inside its session.
  2. VERIFY (fresh read-only agent with `verify: true` so it can execute commands): frame it adversarially — "you did NOT do this work; be suspicious of its self-report. Re-read the task file yourself, inspect the actual code and artifacts, and run whatever tests YOU judge prove or disprove the acceptance criteria." It chooses its own commands; if a command errors it corrects itself and re-runs within its session. Artifact checks use ABSOLUTE paths under the project artifact root — a DIFFERENT directory from the repository, stamped as `project_artifact_root` in the agent's own stage input.
- REMEDIATION IS MANDATORY, NOT OPTIONAL — this is the difference between a workflow that REPORTS problems and one that FIXES them, which is the entire point. Every task MUST follow implement -> verify -> remediate-and-re-verify, exactly as the example shows. A rejected implement or a verifier that returns anything other than accepted/noop is NOT the end of that task: feed the verifier's VERBATIM findings to a fresh write agent ("fix exactly what they name, do not re-argue them"), then re-verify, up to 3 attempts total. Only after the last attempt still fails do you record the task as blocked with the evidence. A script that runs each task once and records the failure is INCOMPLETE and will be rejected — the tasks must actually be implemented.
- Retry prompts carry the COMPLETE implementation and verifier envelope structure plus the original goal. Preserve every finding, blocker, status, changed-file claim, and tool-evidence field verbatim. Only commands_run[*].output_summary may be bounded: share a 4,000-character budget across those strings in each envelope and mark truncation explicitly. Never reduce findings to a wrapper summary, add constraints, or argue about whether a finding is fair.
- Never edit an existing artifact instance to satisfy a check; produce new artifacts through the real pipeline.
- An honest block naming a real gap is success; fabricated acceptance is failure. The runtime gates independently validate patches, no-op proofs, and test evidence — do not try to outsmart them; they are on your side.
- Deterministic code only (no Math.random, no Date.now); pass any needed timestamps via prompts.
- MANDATORY after all task work, before returning: run BOTH mandatory reviews as read-only critic map→reduce contracts, never as one monolithic agent and never with write mode:
  1. ADVERSARIAL REVIEW: map over every accepted task exactly once with `w.parallel` or `w.fanout`, `tier: 'critic'`, `itemKind: 'review_map'`, and `reviewContract: { kind: 'adversarial_findings', stage: 'map', ... }`. Each map source item MUST name exactly one accepted canonical task id in `canonical_task_ids`. Then run `w.reduce` with `tier: 'critic'` and `reviewContract: { kind: 'adversarial_findings', stage: 'reduce_final', sourceMapCallIds: [...], preserveMapFindings: true, accountingField: 'adversarial_findings', maxInputBytes: 48000 }`. The reducer sees only compact map findings, preserves every map finding verbatim, and may ADD cross-task contradictions.
  2. SOURCE-COVERAGE AUDIT: same map→reduce shape using `reviewContract.kind: 'uncovered_requirements'` and final `accountingField: 'uncovered_requirements'`. Map reviewers compare source requirements/task coverage per accepted task; the reducer preserves every map finding and adds cross-task/source gaps.
  Review map/reduce calls must run AFTER all implementation, remediation, and verification work. Map calls must bound findings (`maxFindingsPerItem`); reducers must declare bounds (`maxInputBytes` or `maxFindingsPerReduce`). If findings are too large, chunk-reduce first, then final reduce. The runtime rejects skipped tasks, duplicate task coverage, write-mode reviews, non-critic reviews, unbounded reducers, and dropped findings.
- Return {
    accepted: [...taskIds],
    blocked: [{ taskId, reason }],
    adversarial_findings: [ '<finding or empty>' ],
    uncovered_requirements: [ '<requirement no task covers, or empty>' ],
    notes: '<short honest summary>'
  } accounting for EVERY task id exactly once across accepted+blocked; adversarial_findings and uncovered_requirements MUST come from their final reducers, never invented or omitted."#;

const V3_AUTHOR_TASK_TEMPLATE: &str = r#"Author the complete workflow.js orchestration script for this decomposed task set. INVESTIGATE BEFORE WRITING — you have READ tools (Read, Grep, Glob); you have NO shell and must NOT run commands or create/modify ANY files. Your ONLY deliverable is the result envelope.

Required investigation (do it; cite the files you actually read in evidence):
1. READ the source requirements document(s) under the source roots below, and EVERY task file listed.
2. Inspect the repository tree with Glob/Read (key directories, the files each task declares); distrust any existing status/acceptance documents — verify against the live tree.
3. For each task, extract its EXACT declared target files, dependencies, acceptance criteria, and artifact contracts — honor them verbatim, never invent paths. Use canonical task ids verbatim in taskIds.
4. Decide sequential vs parallel FROM THE TASK DATA: tasks editing shared files or linked by dependencies run sequentially; only genuinely independent tasks may batch.

Then write the script per the dialect reference and SELF-CHECK before returning:
- every canonical task id appears in EXACTLY ONE INITIAL write agent() call's taskIds with that task's declared target files (never one umbrella call claiming many tasks); bounded remediation calls repeat only that same task id and target ownership;
- a task that is already implemented still gets its write agent — instruct that agent to return the typed no-op (status noop, idempotent_noop true, task_coverage evidence) when it verifies nothing needs changing; NEVER make cosmetic edits just to show work;
- EVERY task has a remediation path: after its verifier, a bounded loop (max 3 attempts) that re-runs a write agent with the verifier's verbatim findings and re-verifies, before recording blocked. A script without remediation does not implement the tasks and is incomplete;
- write agents are told to prove their change by running tests IN-SESSION; only add focusedTests commands you verified against the repo (a wrong package or module name fails the gauntlet — when unsure, omit them);
- the two mandatory map→reduce reviews are present after all work, read-only, critic-tier throughout, cover every accepted task exactly once, preserve map findings into reducers, and return adversarial_findings/uncovered_requirements from those reducers;
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
        ![
            "{repo_root}",
            "{source_roots}",
            "{task_paths}",
            "{retry_feedback}",
            "{learning_context}",
            "{reference}"
        ]
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
