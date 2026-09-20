// Reference handed to the author agent. Generic by construction: it documents
// the dialect, never the fixture domain. Prompt copy is byte-identical to what
// the binary shipped: agents parse structured output against these strings.

use crate::task_universe::WorkflowV2TaskUniverse;

pub const V3_PRIMITIVE_REFERENCE: &str = r#"WORKFLOW SCRIPT DIALECT (v3)

Shape — top-level script, exactly like this (no wrapper function):

  export const meta = { name: '<kebab-name>', description: '<one line>', schema: 2, phases: [{ title, detail }] }

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
  // The waves come from the brief's EXECUTION WAVES section, which the host
  // computed. Copy that grouping exactly — task ids only.
{example_waves}
  const acceptedTaskIds = []
  const blockedTasks = []
  const byId = (id) => tasks.find((t) => t.id === id)
  // Each task's implementation envelope, filled in by the wave batch below and
  // read by the per-task verify/remediate loop after it.
  const implOf = {}

  // IMPLEMENT BY WAVE. Every task in one wave goes in ONE agents([...]) call —
  // that is the whole point of the waves, and a wave of three issued as three
  // agent() calls costs three times the wall clock for no added safety. A wave
  // of ONE is still one agents([...]) call with one spec; do not special-case it.
  //
  // ONE BOUND: a single write call may not claim HALF OR MORE of all the tasks
  // in the universe — that is how umbrella id-stuffing is detected, and a batch
  // large enough to trip it fails validation however legitimate its grouping.
  // If a wave is that large, split it across several agents([...]) calls in the
  // same phase; they still run concurrently and each claims fewer ids.
  for (const wave of waves) {
    const batch = await agents(
      wave.map((id) => ({
        prompt: `Implement ${id} per ${byId(id).file}. Resolve repository paths against the repository_root in YOUR OWN stage input — the host stamps your isolated checkout there. NEVER paste an absolute repository path into this prompt. Re-inspect the current state FIRST — if the work is genuinely already done, return the typed no-op. Prove your change with tests you run yourself.`,
        label: `implement-${id.toLowerCase()}`,
        taskIds: [id],
        targetFiles: byId(id).targetFiles,
      })),
      { write: true, maxParallelism: wave.length },
    )
    // Per-item results live in the batch envelope as two TOP-LEVEL arrays that
    // carry different things: `batch.outcomes[]` is the contract view
    // ({ item_id, status, canonical_task_ids, summary }) and `batch.items[]` is
    // the work view (status, commands_run, files_changed, evidence,
    // residual_gaps). There is NO `batch.data` wrapper: the host spreads the
    // fan-out's data at the top level, so a `batch.data.outcomes` read is
    // always empty and every task then looks unimplemented. Remediation needs
    // the work view joined to the task identity, so read the branches ONLY
    // through the runtime global `outcomesOf(batch)`: it joins each outcome
    // with its item and finds the arrays wherever the host puts them. Match
    // identity on canonical_task_ids and never on array position alone.
    const branches = outcomesOf(batch)
    for (const id of wave) {
      const branch = branches.find((o) => (o.canonical_task_ids || []).includes(id))
      // A wave item that produced no outcome is a host-side failure, not an
      // accepted task: record it so the loop below remediates rather than
      // silently treating a missing entry as done.
      implOf[id] = branch || { status: 'failed', summary: `no outcome returned for ${id} in its wave batch` }
    }
  }

  // VERIFY AND REMEDIATE PER TASK. These stay per-task and sequential: each
  // one's budget follows its own verifier's findings, so they cannot share a
  // batch.
  for (const t of tasks) {
    let impl = implOf[t.id]
    // A verifier that demotes the task is not the end: feed its verbatim findings
    // to a fresh write agent and re-verify, up to 6 attempts, then record blocked.
    let check = await agent(`You did NOT implement ${t.id} — be suspicious of its self-report. Re-read ${t.file}, inspect the actual code, and run whatever tests YOU judge prove or disprove the acceptance criteria.`, { label: `verify-${t.id.toLowerCase()}`, verify: true, taskIds: [t.id] })
    // Budget follows PROGRESS, not a flat count: it extends past the base
    // attempts only while the FIRST verifier's gap set is still shrinking, and
    // stops on a plateau. Do not replace this with a fixed bound.
    const budget = remediationBudget()
    // `usable`/`accepted` are runtime globals (see the rule below); never a
    // local isAccepted that re-derives them from the envelope.
    for (let attempt = 2; budget.shouldContinue(attempt - 1, check, impl) && (!usable(impl) || !accepted(check)); attempt += 1) {
      const rejectedAttempt = `Implementation envelope:\n${remediationEvidence(impl)}\nVerifier envelope:\n${remediationEvidence(check)}`
      impl = await agent(`Remediate ${t.id}. The previous attempt was REJECTED. Fix exactly what these verbatim implementation and verifier envelopes name; do not re-argue them:\n${rejectedAttempt}\nOriginal goal: implement ${t.id} per ${t.file}. Resolve repository paths against the repository_root in YOUR OWN stage input — never an absolute path written into this prompt. Prove the fix with tests you run yourself.`, { label: `remediate-${t.id.toLowerCase()}-${attempt}`, write: true, taskIds: [t.id], targetFiles: t.targetFiles })
      check = await agent(`You did NOT implement ${t.id} — be suspicious. The previous attempt was rejected with these verbatim findings:\n${rejectedAttempt}\nRe-read ${t.file}, inspect the actual code, and run whatever tests YOU judge prove or disprove the acceptance criteria.`, { label: `verify-${t.id.toLowerCase()}-${attempt}`, verify: true, taskIds: [t.id] })
    }
    if (usable(impl) && accepted(check)) acceptedTaskIds.push(t.id)
    else blockedTasks.push({ taskId: t.id, reason: summarize(check) })
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

  phase('Acceptance')
  // MANDATORY FINAL STAGE — the runtime runs every check in the task set's
  // frozen acceptance-contract.json against the repository as the run left it
  // (host call `acceptance-contract-run`), hands each failing check to the
  // task(s) whose `implements` list names it through the same bounded
  // remediateFindings loop, re-runs ONLY the checks that failed, and records
  // every round. The host will not record the run complete while the final
  // round has a failing check; a script without this call fails pre-flight.
  // Nothing may follow it but the accounting return.
  const acceptance_gate = await acceptance({ taskFileFor: (id) => (tasks.find((t) => t.id === id) || {}).file, targetFilesFor: (id) => (tasks.find((t) => t.id === id) || {}).targetFiles })

  return {
    accepted: acceptedTaskIds,
    blocked: blockedTasks,
    adversarial_findings,
    uncovered_requirements,
    review_remediation,
    acceptance_gate,
    notes: 'short honest summary',
  }
  // Your own small helpers, defined at the top of the script (NOT a status
  // predicate — accepted(env)/usable(env) are runtime globals, see the rule):
  //   remediationEvidence(env) -> JSON.stringify the complete envelope with every
  //      finding intact, but share a 4,000-character budget across only its
  //      commands_run[*].output_summary strings and mark any truncation
  //   summarize(env)  -> short text used only for final blocked accounting
  //      (read env.summary; the full records are under env.result.*)
  //   boundedEvidenceFor(taskId) -> a compact, bounded evidence array for a task
  //      id (its accepted claims/artifacts) — the reviewers falsify against it

Statements run at the top level: bare phase()/log() (no await needed), `await agent(...)`, and a final top-level `return`.

Primitives:
- await agent(prompt, opts) -> result envelope { ...data keys spread at the top level, status, summary, result,
  files_changed, commands_run, evidence, residual_gaps, artifacts }  // MUST be awaited
  There is no `data` wrapper: what the agent returned in `data` sits at the top level of the envelope
  (a fan-out's `items`/`outcomes` are `batch.items`/`batch.outcomes`), and `result` is the typed aggregate
  (status, summary, evidence, commands_run, files_changed, residual_gaps, artifacts, task_coverage). The five
  top-level arrays after `result` are COMPACT MIRRORS of `result.*` for reporting: `files_changed` is paths,
  `commands_run` is { command, status }, `residual_gaps` is { id, severity }; read `result.*` for the full records.
  opts: {
    label: '<short-kebab-label>'          // required; call ids derive from it deterministically
    write: true,                          // spawn a WRITE agent in a sealed worktree through the write gauntlet
    taskIds: ['<canonical task id>'],     // required when write:true
    targetFiles: ['path/one.ext'],        // LITERAL repo-relative file paths ONLY (never descriptions); the write agent owns exactly these
    verify: true,                         // REQUIRED on per-task verifiers: routes the agent through the host
                                          // verification machinery WITH command execution. Without it (or a
                                          // non-empty focusedTests) the agent has NO shell and any test runs
                                          // it claims are downgraded to inspection — hollow verification.
    focusedTests: ['test command'],       // ONLY commands the task file itself DECLARES, copied verbatim;
                                          // omit when it declares none. Never invent or widen one — a command
                                          // naming something the project does not have fails the run.
                                          // If given, must match >0 tests. On a read-only agent a non-empty
                                          // list routes through the verification machinery like verify:true.
    artifacts: ['relative/artifact.path'],// ONLY files this task must PRODUCE WITH CONTENT as its deliverables:
                                          // a report, a generated spec, a data output the task itself populates.
                                          // The host checks every listed path on return. Absent or zero bytes
                                          // FAILS the branch; parses but holds no records is flagged for the
                                          // verifier. A file the task file says is "mutated only by code paths",
                                          // "written by the code under test", "never hand-edited", or that a
                                          // LATER task populates is NOT an artifact of this task —
                                          // put it in targetFiles if the task may edit it, otherwise nowhere.
                                          // WRONG: a schema-migration task listing the data store its new schema
                                          // will hold; that store is legitimately empty until the task that loads
                                          // it runs. RIGHT for that task: its migration report, if the task file
                                          // declares one.
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
- await acceptance({ taskFileFor, targetFilesFor }) -> { complete, contract_present, rounds, failing, unowned_failing, passed, record_path }
  THE MANDATORY FINAL STAGE. Runs every check in the task set's frozen acceptance-contract.json against the
  repository as the run left it, through the host (call `acceptance-contract-run`, never an agent). Failing
  checks are routed to the tasks whose `implements` list names them through the same bounded
  remediateFindings fix + re-verify loop the reviews use (targetFilesFor supplies the files each task owns),
  then ONLY the checks that failed re-run; at most 3 rounds. A failing check no task implements is
  reported as a set-level gap and never forced green. The host derives the run's terminal status from
  the final round: any failing check means the run ends `needs review`, not complete.

Rules the script must follow:
- AWAIT EVERY agent(), agents(), pipeline(), and phase-with-body call. Never fire-and-forget real work: a workflow that returns while work is pending FAILS the run with a dropped-call error. (Bare phase()/log() markers are the only calls that need no await.)
- Dependency order always. Two tasks are INDEPENDENT when neither declares the other in its dependencies (directly or transitively) AND their declared target files do not overlap. Independent tasks MUST be batched into one `await agents([...])` call — batching is the expected shape for them, not an optional optimisation, and a script that runs independent tasks one at a time wastes hours of wall clock for no safety gain. Everything else runs sequentially. Parallel writes outside `agents()` are forbidden. Apply the same test to REMEDIATION: independent tasks being remediated in the same round batch together too.
- PER TASK, TWO STAGES, GOAL-ORIENTED PROMPTS — agents are capable sessions with their own tools; give them goals and context, never command scripts to obey:
  1. IMPLEMENT (write agent): give it the task file PATH and the goal, and tell it to resolve repository paths against the `repository_root` in its OWN stage input. DO NOT write an absolute repository path into the prompt. A write agent runs in an isolated git checkout and the host stamps that checkout as its `repository_root`; a literal path in the prompt names the CANONICAL tree instead, and the agent then edits the real repository directly — outside its worktree, so its patch is empty, the write coordinator sees nothing to inspect, and every ownership and overlap guard is bypassed. This is the same discipline as the artifact root below, for the same reason; for artifact work tell it to use `project_artifact_root` from its OWN stage input (the host stamps it there — never guess or invent an artifact path yourself). Tell it to READ the task file and RE-INSPECT the current repo/artifact state FIRST — if the work is genuinely already done it returns the typed no-op (status noop, idempotent_noop true, task_coverage evidence) instead of redoing or cosmetically editing anything; the workflow must be safe to re-run. It decides how to implement and how to prove it, runs its own tests, and fixes its own command mistakes inside its session.
  2. VERIFY (fresh read-only agent with `verify: true` so it can execute commands): frame it adversarially — "you did NOT do this work; be suspicious of its self-report. Re-read the task file yourself, inspect the actual code and artifacts, and run whatever tests YOU judge prove or disprove the acceptance criteria." It chooses its own commands; if a command errors it corrects itself and re-runs within its session. Artifact checks use ABSOLUTE paths under the project artifact root — a DIFFERENT directory from the repository, stamped as `project_artifact_root` in the agent's own stage input.
- REMEDIATION IS MANDATORY, NOT OPTIONAL — this is the difference between a workflow that REPORTS problems and one that FIXES them, which is the entire point. Every task MUST follow implement -> verify -> remediate-and-re-verify, exactly as the example shows. A rejected implement or a verifier that returns anything other than accepted/noop is NOT the end of that task: feed the verifier's VERBATIM findings to a fresh write agent ("fix exactly what they name, do not re-argue them"), then re-verify, up to 6 attempts total. Only after the last attempt still fails do you record the task as blocked with the evidence. A script that runs each task once and records the failure is INCOMPLETE and will be rejected — the tasks must actually be implemented.
- Retry prompts carry the COMPLETE implementation and verifier envelope structure plus the original goal. Preserve every finding, blocker, status, changed-file claim, and tool-evidence field verbatim. Only commands_run[*].output_summary may be bounded: share a 4,000-character budget across those strings in each envelope and mark truncation explicitly. Never reduce findings to a wrapper summary, add constraints, or argue about whether a finding is fair.
- Never edit an existing artifact instance to satisfy a check; produce new artifacts through the real pipeline.
- An honest block naming a real gap is success; fabricated acceptance is failure. The runtime gates independently validate patches, no-op proofs, and test evidence — do not try to outsmart them; they are on your side.
- Deterministic code only (no Math.random, no Date.now); pass any needed timestamps via prompts.
- REVIEW REMEDIATION CONTRACT. A call that acts on review findings declares
  `remediationContract` in its options, and the host validates every field. Omit
  or misname one and the draft is rejected:
      remediationContract: {
        stage: 'remediate',              // or 'verify' — no other value
        taskId: '<the ONE canonical task this call remediates>',
        sourceReduceCallIds: ['<id of a planned reduce_final review call>'],
        maxRounds: 3,                    // 1..=3
        round: 1,                        // 1..=maxRounds
      }
  `sourceReduceCallIds` must hold the EXACT ids you passed to `w.reduce` for the
  final review reduces, and those reduces must be planned BEFORE the remediation
  that names them. Every `stage: 'remediate'` call needs a later
  `stage: 'verify'` call for the same `taskId`, and the verifier is read-only —
  it must not set a write mode.
- READ REVIEW FINDINGS ONLY THROUGH `reviewFindings(reduced)`. The host computes
  each review's finding set itself -- every map branch's findings, attributed to
  the task that branch reviewed, merged with the reducer's own cross-task
  findings -- and attaches it to the reduce result. `reviewFindings` returns
  that attachment. The accounting you return must be exactly what
  `reviewFindings` returned for each final reducer: the host compares the two,
  and a script that filters, re-extracts or invents findings between reading
  and reporting them is refused after every task was implemented, verified and
  reviewed.
- DO NOT WRITE YOUR OWN RESULT PREDICATES. `accepted(env)`, `usable(env)`,
  `outcomesOf(batch)` and `reviewFindings(reduced)` are runtime globals carrying
  the host's own rules: `usable`
  is accepted with changed files or commands run, or a typed no-op with
  task_coverage evidence, and `outcomesOf` finds a fan-out's outcomes wherever
  they sit. A hand-rolled version that disagrees does not fail the run, it loops
  it — one live run spent every remediation round redoing work already done,
  and another remediated after EVERY accepted verify because its own
  `isAccepted` required `env.files_changed`/`env.commands_run` to be non-empty
  while the evidence sat under `env.result`. So: every status predicate MUST be
  `accepted(env)` or `usable(env)` — `usable` for a write agent's envelope,
  `accepted` for a verifier's verdict, as in
      if (usable(impl) && accepted(check)) acceptedTaskIds.push(t.id)
  and a script MUST NOT define its own (`isAccepted`, `isUsable`, `passed`,
  `ok`, ...) that combines `env.status` with `files_changed`/`commands_run`
  length checks. The dry-run pre-flight rejects a script whose own
  accept/usable/ok/passed-named function reads `.files_changed` or
  `.commands_run`; reporting helpers (`summarize`, `boundedEvidenceFor`) may
  read them freely.
  Read a fan-out's branches ONLY through `outcomesOf(batch)`, never through
  the raw `batch.outcomes`/`batch.items` arrays directly (and never through a
  `batch.data.*` path — there is no such wrapper, so that read is always
  empty): a branch is reported in two places and the raw outcome can carry
  empty files_changed/commands_run while the work is recorded beside it, so
  the raw view says a finished branch proved nothing.
- THE REVIEW PRIMITIVES TAKE THE ID FIRST. Every `w.*` call is `w.method(id, ...)`
  with a non-empty string id as its FIRST POSITIVE ARGUMENT; the options object is
  the argument AFTER it. The examples above use the prelude helpers, so these two
  are the shapes the mandatory reviews below need:
      const map = await w.parallel('adversarial-map', mapItems, { tier: 'critic', itemKind: 'review_map', reviewContract: { ... } })
      const reduced = await w.reduce('adversarial-reduce-final', { tier: 'critic', reviewContract: { ... } })
  Passing the spec alone — `w.reduce({ id, ... })` — makes the id an object and the
  run dies on `w.reduce requires a non-empty string id`.
- MANDATORY after all task work, before returning: run BOTH reviews. `adversarialReview(acceptedTaskIds, { evidenceFor })` and `coverageAudit(acceptedTaskIds, { evidenceFor })` are runtime globals that ALREADY emit the exact contract the host validates — one critic map item per accepted task, a `reduce_final` reducer naming its map in `sourceMapCallIds` with `preserveMapFindings: true`, the right `accountingField`, and bounds. USE THEM, as the example above does, and pass their findings to `remediateFindings`. Hand-rolling the same graph out of `w.parallel`/`w.reduce` is allowed but is where drafts fail: every field below is then yours to get right, and a single wrong id is a rejection. If you do hand-roll it, never as one monolithic agent and never with write mode:
  1. ADVERSARIAL REVIEW: map over every accepted task exactly once with `w.parallel` or `w.fanout`, `tier: 'critic'`, `itemKind: 'review_map'`, and `reviewContract: { kind: 'adversarial_findings', stage: 'map', ... }`. Each map source item MUST name exactly one accepted canonical task id in `canonical_task_ids`. Then run `w.reduce` with `tier: 'critic'` and `reviewContract: { kind: 'adversarial_findings', stage: 'reduce_final', sourceMapCallIds: [...], preserveMapFindings: true, accountingField: 'adversarial_findings', maxInputBytes: 48000 }`. The reducer sees only compact map findings, preserves every map finding verbatim, and may ADD cross-task contradictions.
  2. SOURCE-COVERAGE AUDIT: same map→reduce shape using `reviewContract.kind: 'uncovered_requirements'` and final `accountingField: 'uncovered_requirements'`. Map reviewers compare source requirements/task coverage per accepted task; the reducer preserves every map finding and adds cross-task/source gaps.
  Review map/reduce calls must run AFTER all implementation, remediation, and verification work. Map calls must bound findings (`maxFindingsPerItem`); reducers must declare bounds (`maxInputBytes` or `maxFindingsPerReduce`). If findings are too large, chunk-reduce first with `reviewContract.stage: 'reduce_chunk'` — each chunk reducer covering its map calls exactly once — then the `reduce_final` reducer names those chunk reducers in `sourceMapCallIds`. The runtime rejects skipped tasks, duplicate task coverage, write-mode reviews, non-critic reviews, unbounded reducers, and dropped findings.
- ACCEPTANCE IS THE FINAL STAGE, AND IT IS MANDATORY. After remediateFindings, call
  `const acceptance_gate = await acceptance({ taskFileFor, targetFilesFor })` exactly once, unconditionally,
  and return immediately after it: no agent, review, or remediation call of your own may follow it, and
  it may not run before both final review reduces. Declare `schema: 2` in `meta` — it marks a script
  written under this rule, and the dry-run pre-flight rejects a `schema: 2` script whose last stage is not
  the acceptance call, or a fresh script that omits the marker.
- Return {
    accepted: [...taskIds],
    blocked: [{ taskId, reason }],
    adversarial_findings: [ '<finding or empty>' ],
    uncovered_requirements: [ '<requirement no task covers, or empty>' ],
    acceptance_gate: <exactly what acceptance() returned>,
    notes: '<short honest summary>'
  } accounting for EVERY task id exactly once across accepted+blocked; adversarial_findings and uncovered_requirements MUST come from their final reducers, never invented or omitted."#;

pub(super) const V3_AUTHOR_TASK_TEMPLATE: &str = r#"Author the complete authored-workflow.js orchestration script for this decomposed task set. THE DIALECT REFERENCE BELOW IS THE ONLY EXAMPLE THERE IS — do not go looking through the project for another one. Nothing under `.archon/workflows/` is an example of what you are writing: those are finished runs' records and results, one of them is a YAML plan record, and the metadata beside them is large enough to cost you the context you need for this job. The REST of `.archon/` is ordinary project material — agent and skill definitions, docs, tools, and the project artifact root — and you read it exactly as you would any other directory. INVESTIGATE BEFORE WRITING — you have READ tools (Read, Grep, Glob); you have NO shell and must NOT run commands or create/modify ANY files. Your ONLY deliverable is the result envelope.

Required investigation (do it; cite the files you actually read in evidence):
1. THE TASK UNIVERSE IN YOUR STABLE CONTEXT IS THE AUTHORITATIVE TASK SET. Canonical ids, file names, dependencies, declared target files, deliverable contracts and focused tests are already there — take them from it. Read the source requirements document(s) once for intent, and open a task file only for detail the universe does not carry. Do not re-derive from disk what the universe already states, and never re-read a file you have already read.
2. Inspect the repository tree with Glob/Read (key directories, the files each task declares); distrust any existing status/acceptance documents — verify against the live tree. Stay out of `.archon/workflows/`: it holds previous runs, not the code the tasks describe. The rest of `.archon/` is fair game and often necessary.
3. Take each task's declared target files, dependencies and artifact contracts FROM THE TASK INDEX; read a task body only when its detailed criteria are necessary — honor them verbatim, never invent paths. Use canonical task ids verbatim in taskIds.
4. USE THE EXECUTION WAVES GIVEN BELOW. They are computed by the host from the same declared `depends_on` and target-file data you are reading, so they are fact, not a suggestion — do not re-derive them and do not second-guess them. Waves run in order; every task inside one wave group runs together in ONE `await agents([...])` call. Serialising a group that the waves batch is a defect, and so is batching across waves.

Then write the script per the dialect reference and SELF-CHECK before returning:
- every canonical task id appears in EXACTLY ONE INITIAL write agent() call's taskIds with that task's declared target files (never one umbrella call claiming many tasks); bounded remediation calls repeat only that same task id and target ownership;
- a task that is already implemented still gets its write agent — instruct that agent to return the typed no-op (status noop, idempotent_noop true, task_coverage evidence) when it verifies nothing needs changing; NEVER make cosmetic edits just to show work;
- EVERY task has a remediation path: after its verifier, a bounded loop (max 6 attempts) that re-runs a write agent with the verifier's verbatim findings and re-verifies, before recording blocked. A script without remediation does not implement the tasks and is incomplete;
- write agents are told to prove their change by running tests IN-SESSION; the ONLY focusedTests you may pass are the commands the task itself declares, listed verbatim under DECLARED FOCUSED TESTS below — copy them character for character. You have no shell, so you cannot check a command of your own; NEVER invent one, never widen a declared one into a broader filter, and never pattern-match a name out of the repository tree. An invented command fails the gauntlet or drags in work the task never owned. A task that declares none gets no focusedTests at all: omit the option and let its agent choose;
- SCOPE EVERY TEST COMMAND TO WHAT THE TASK CHANGED. Use the project's own tooling to run the package, module or suite the task touches — never the whole repository. A task editing one component does not need the entire tree built and tested to prove itself, and on a large project that difference is hours per task, repeated for every task and every remediation attempt. Tell the write agent the same thing: prove the change with the narrowest command that actually exercises it, and widen only if the narrow one cannot;
- the two mandatory map→reduce reviews are present after all work, read-only, critic-tier throughout, cover every accepted task exactly once, preserve map findings into reducers, and return adversarial_findings/uncovered_requirements from those reducers;
- the LAST stage is `const acceptance_gate = await acceptance({ taskFileFor, targetFilesFor })`, called once, unconditionally, after remediateFindings and both reviews, with nothing but the accounting `return` after it; `meta` declares `schema: 2`. The host runs the task set's frozen acceptance checks there and will not record the run complete while one fails — a script without this stage is rejected at pre-flight;
- meta.phases matches the phase() calls; the accounting return covers every task id exactly once and carries `acceptance_gate`;
- the script text must not contain confirmation questions or the phrases "restored context"/"previous session summary".

Reply with the standard JSON result envelope; put ONLY the complete script text in data.workflow_js (no fences) — workflow_js must sit INSIDE data. Include evidence entries naming the files you read.

Repository root: {repo_root}
Source requirement roots: {source_roots}
Task files (read only when index fields are insufficient; fingerprints identify source changes):
{task_paths}

DECLARED FOCUSED TESTS — each task file's own verified commands. The task
authors ran these; you cannot run anything, so these are the only test commands
you may put in a focusedTests option, and they go in exactly as written:
{declared_focused_tests}

Execution waves (computed by the host from declared dependencies and target
files — batch exactly as grouped here):
{task_waves}

{retry_feedback}
WHAT KEEPS GOING WRONG. These are the failures that actually recur, distilled
into rules. They are not history to look up — everything you need is in this
brief:
- A WRITE AGENT THAT CHANGES NOTHING HAS NOT SUCCEEDED. The most common failure
  by far is an implementation call that returns a cheerful summary with no files
  changed and no commands run. Treat an outcome with empty files_changed and
  empty commands_run as FAILED and send it to remediation, unless it is an
  explicit typed no-op (status noop, idempotent_noop true) carrying task_coverage
  evidence that the work was already done. "I reviewed it and it looks fine" is
  not a no-op, it is a miss. That rule IS the runtime global `usable(env)`:
  call it, never re-derive it from the envelope's arrays.
- A TASK IS NOT DONE UNTIL ITS COMPLETION IS RECORDED. Runs repeatedly ended with
  work performed but no accounting entry for it. Every canonical task id must
  appear exactly once in the accounting return, with its real status.
- A NO-OP MUST CARRY ITS PROOF. A task claimed as already-implemented needs the
  evidence that proves it — the file or test output that shows the work exists.
  A bare noop claim is rejected.
- A CALL THAT DIES IN TRANSPORT IS NOT A TASK THAT FAILED. Transport errors are
  by far the most frequent error in practice. Do not record a task as blocked
  because its call was cancelled or dropped; that is a retry, not a verdict.

{curated_lessons}
DIALECT REFERENCE:
{reference}"#;

/// Single-pass placeholder substitution: each `{name}` is looked up once —
/// substituted content is never re-scanned, so run-derived text (learning
/// context, retry errors) cannot inject other placeholders.
pub fn compose_author_brief(values: &[(&str, &str)]) -> String {
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
            "{declared_focused_tests}",
            "{task_waves}",
            "{retry_feedback}",
            "{curated_lessons}",
            "{example_waves}",
            "{reference}"
        ]
        .iter()
        .any(|token| out.contains(token)),
        "author brief has unsubstituted placeholders"
    );
    out
}

/// Every task's declared focused-test commands, rendered for the author brief.
///
/// The author agent has READ tools and no shell, so it can neither verify a
/// test command nor discover one by running anything — and the brief used to
/// hand it task ids, paths and fingerprints only. Left to guess, it guessed:
/// one live run invented a whole-package filter, drew in failures no task in
/// the universe owned, and could neither satisfy nor abandon them for 6h20m.
///
/// The commands it needed were already parsed. Each task file declares them
/// under `## Focused Tests`, narrowly scoped and verified by the author who
/// could run them, and they arrive here as `focused_tests`.
pub fn render_declared_focused_tests(universe: &WorkflowV2TaskUniverse) -> String {
    let rendered = universe
        .tasks
        .iter()
        .filter(|task| !task.focused_tests.is_empty())
        .map(|task| {
            let commands = task
                .focused_tests
                .iter()
                .filter_map(|entry| declared_command(entry))
                .map(|command| format!("  - {command}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("- {} declares:\n{commands}", task.canonical_task_id)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if rendered.is_empty() {
        "<no task declares any focused test; pass no focusedTests at all>".to_string()
    } else {
        rendered
    }
}

/// The command out of one declared bullet.
///
/// A bullet is markdown: the command sits in a backticked span, often followed
/// by prose saying what it proves. The span is the command — passing the prose
/// with it would hand the author a string no shell could run.
fn declared_command(entry: &str) -> Option<String> {
    let trimmed = entry.trim();
    let candidate = match trimmed.split_once('`') {
        Some((_, rest)) => rest.split('`').next().unwrap_or(rest),
        None => trimmed,
    };
    let candidate = candidate.trim();
    (!candidate.is_empty()).then(|| candidate.to_string())
}

#[cfg(test)]
#[path = "v3_author_focused_tests_tests.rs"]
mod focused_tests_tests;

#[cfg(test)]
#[path = "v3_author_envelope_tests.rs"]
mod envelope_tests;

#[cfg(test)]
#[path = "v3_author_artifacts_tests.rs"]
mod artifacts_tests;
