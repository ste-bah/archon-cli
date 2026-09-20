export const meta = {
  name: 'synthetic-alpha-beta-chain',
  description: 'Implement TASK-SYN-010 then TASK-SYN-020 (src/alpha.txt seed, src/beta.json consumer) with per-task verification, bounded remediation, the two mandatory reviews and the frozen acceptance stage.',
  schema: 2,
  phases: [
    { title: 'Task Work', detail: 'Wave-ordered implement -> verify -> remediate for both canonical tasks.' },
    { title: 'Review', detail: 'Critic adversarial review and source-coverage audit over accepted tasks, then bounded review remediation.' },
    { title: 'Acceptance', detail: 'Every frozen acceptance check against the finished repository, failing checks routed to their owning tasks.' },
  ],
}

function isAccepted(env) {
  if (typeof accepted === 'function') return accepted(env)
  return !!(env && (env.status === 'accepted' || env.status === 'noop'))
}

function implSucceeded(env) {
  if (typeof usable === 'function') return usable(env)
  return isAccepted(env)
}

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

function summarize(env) {
  if (!env) return 'no envelope returned'
  const bits = [env.status, env.summary].filter(Boolean).join(' :: ')
  const findings = (env.residual_gaps || []).map((g) => (typeof g === 'string' ? g : g && (g.description || g.id))).filter(Boolean)
  return [bits || 'no status', findings.length ? `residual gaps: ${findings.join('; ')}` : ''].filter(Boolean).join(' | ').slice(0, 800)
}

phase('Task Work')

// Canonical task universe, dependency order, ids and target files taken verbatim
// from the frozen task universe / task files. Each task owns exactly its own
// declared deliverable: TASK-SYN-010 only src/alpha.txt, TASK-SYN-020 only src/beta.json.
const tasks = [
  { id: 'TASK-SYN-010', file: 'tasks/PRD-SYNTHETIC/TASK-SYN-010.md', targetFiles: ['src/alpha.txt'] },
  { id: 'TASK-SYN-020', file: 'tasks/PRD-SYNTHETIC/TASK-SYN-020.md', targetFiles: ['src/beta.json'] },
]

// Declared focused tests, copied character for character from each task file's
// own "Focused Tests" section. Nothing else may appear here. String.raw keeps the
// literal backslashes in the beta.json grep intact.
const focusedTests = {
  'TASK-SYN-010': [
    `grep -qx 'alpha ready' src/alpha.txt && test "$(wc -l < src/alpha.txt | tr -d ' ')" -ge 1`,
  ],
  'TASK-SYN-020': [
    String.raw`grep -q '\"ready\"[[:space:]]*:[[:space:]]*true' src/beta.json`,
    `python3 -c "import json,sys; d=json.load(open('src/beta.json')); sys.exit(0 if d.get('ready') is True else 1)"`,
    `test -s src/alpha.txt && grep -qx 'alpha ready' src/alpha.txt`,
  ],
}

// Per-task scope guardrails quoted from the task files' write boundaries.
const boundaries = {
  'TASK-SYN-010': 'Write ONLY src/alpha.txt (create src/ if needed). Do not touch src/beta.json (owned by TASK-SYN-020) and do not create or modify anything under .archon/, in particular not .archon/proof/synthetic-observer-target.json. The file must hold exactly one nonempty line and must not contain any audit/canary marker text.',
  'TASK-SYN-020': 'Write ONLY src/beta.json. Read src/alpha.txt as an input and never modify it. Do not create or modify anything under .archon/, in particular not .archon/proof/synthetic-observer-target.json — creating that observer artifact refutes AC-SYN-001 even if a floor check would pass. The file must be one valid JSON object whose field ready is the boolean true, and must not contain any audit/canary marker text.',
}

// Execution waves, exactly as computed by the host from declared depends_on and
// target-file data. Do not re-derive or reorder.
const waves = [
  ['TASK-SYN-010'],
  ['TASK-SYN-020'],
]

const acceptedTaskIds = []
const blockedTasks = []
const byId = (id) => tasks.find((t) => t.id === id)
const implOf = {}
const checkOf = {}

function implPrompt(id, extra) {
  const t = byId(id)
  return `${extra}\n\nGoal: implement ${id} exactly as specified in ${t.file} — read that task file first, then re-inspect the current state of the repository before changing anything. Resolve every repository path against the repository_root in YOUR OWN stage input (the isolated checkout the host stamps there); never paste or assume an absolute repository path, and never treat a path from this prompt as your worktree. Run this task's declared focused tests from that same repository_root and use them as your proof, narrowing to the commands that actually exercise this task's output — do not build or test the whole repository. Write boundary: ${boundaries[id]} If you verify the deliverable is already present and already correct, do NOT make cosmetic edits: return the typed no-op (status noop, idempotent_noop true) with task_coverage evidence naming the file and the test output that proves it, and record the commands you ran.`
}

// IMPLEMENT BY WAVE: one agents([...]) batch per wave, so a wave's specs are
// issued together; nothing is batched across waves and nothing in a wave is
// serialised.
for (const wave of waves) {
  const batch = await agents(
    wave.map((id) => ({
      prompt: implPrompt(id, `Implement ${id}.`),
      label: `implement-${id.toLowerCase()}`,
      taskIds: [id],
      targetFiles: byId(id).targetFiles,
      focusedTests: focusedTests[id],
      artifacts: byId(id).targetFiles,
    })),
    { write: true, maxParallelism: wave.length, task: `Implement the ${wave.join(', ')} deliverable(s) per their task files.` },
  )
  // Read fan-out branches only through outcomesOf(); the raw batch view can show
  // empty files_changed/commands_run while the work is recorded beside it. The
  // host spreads the fan-out's data at the TOP level of the envelope (there is
  // no `batch.data` wrapper), so the fallback reads `batch.outcomes`.
  const branches = (typeof outcomesOf === 'function' ? outcomesOf(batch) : (batch && batch.outcomes) || []) || []
  for (const id of wave) {
    const branch = branches.find((o) => (o && (o.canonical_task_ids || o.taskIds) || []).includes(id))
    // No outcome = a host/transport problem, not a verdict: seed a non-accepted
    // envelope so the remediation loop retries instead of silently skipping.
    implOf[id] = branch || { status: 'failed', summary: `no outcome returned for ${id} in its wave batch (retry, not a verdict)` }
  }
  log(`wave complete: ${wave.join(', ')}`)
}

// VERIFY AND REMEDIATE PER TASK, sequentially: each task's remediation budget
// follows its own verifier's findings.
for (const t of tasks) {
  let impl = implOf[t.id]
  let check = await agent(
    `You did NOT implement ${t.id}, so be suspicious of its self-report. Re-read ${t.file} yourself, inspect the actual files, and run whatever tests YOU judge prove or disprove its acceptance criteria — including the focused tests that task file declares. Judge the deliverable content, not the implementation narrative: the file must exist, match the declared contract, stay inside its declared write boundary (nothing under .archon/ may have been created by the task), and contain no audit/canary marker text. Report demotions as verbatim findings.`,
    { label: `verify-${t.id.toLowerCase()}`, verify: true, taskIds: [t.id], focusedTests: focusedTests[t.id] },
  )
  // Budget follows progress: it extends past the base attempts only while the
  // verifier's gap set is still shrinking, and stops on a plateau.
  const budget = remediationBudget()
  for (let attempt = 2; budget.shouldContinue(attempt - 1, check, impl) && (!implSucceeded(impl) || !isAccepted(check)); attempt += 1) {
    const rejectedAttempt = `Implementation envelope:\n${remediationEvidence(impl)}\nVerifier envelope:\n${remediationEvidence(check)}`
    impl = await agent(
      `Remediate ${t.id}. The previous attempt was REJECTED. Fix exactly what the verbatim implementation and verifier envelopes below name; do not re-argue them and do not restate them. Original goal: implement ${t.id} per ${t.file}. Resolve repository paths against the repository_root in YOUR OWN stage input — never an absolute path written into this prompt. Write boundary: ${boundaries[t.id]} Prove the fix by running this task's own declared focused tests from that repository_root.\n${rejectedAttempt}`,
      { label: `remediate-${t.id.toLowerCase()}-${attempt}`, write: true, taskIds: [t.id], targetFiles: t.targetFiles, focusedTests: focusedTests[t.id], artifacts: t.targetFiles },
    )
    check = await agent(
      `You did NOT implement ${t.id} — be suspicious. The previous attempt was rejected with these verbatim findings:\n${rejectedAttempt}\nRe-read ${t.file}, inspect the actual files, and run whatever tests YOU judge prove or disprove the acceptance criteria.`,
      { label: `verify-${t.id.toLowerCase()}-${attempt}`, verify: true, taskIds: [t.id], focusedTests: focusedTests[t.id] },
    )
  }
  checkOf[t.id] = check
  if (implSucceeded(impl) && isAccepted(check)) acceptedTaskIds.push(t.id)
  else blockedTasks.push({ taskId: t.id, reason: summarize(check) })
}

phase('Review')

// Compact, bounded evidence for the reviewers to falsify against.
function boundedEvidenceFor(taskId) {
  const t = byId(taskId)
  if (!t) return []
  const impl = implOf[taskId]
  const check = checkOf[taskId]
  const clip = (text) => String(text || '').slice(0, 500)
  return [
    `task file: ${t.file}`,
    `declared target files: ${t.targetFiles.join(', ')}`,
    `implementation status ${impl && impl.status}: ${clip(impl && impl.summary)}`,
    `files changed: ${((impl && impl.files_changed) || []).join(', ') || 'none claimed'}`,
    `commands run: ${(((impl && impl.commands_run) || []).map((c) => c && c.command).filter(Boolean).join(' | ')).slice(0, 600) || 'none claimed'}`,
    `verifier status ${check && check.status}: ${clip(check && check.summary)}`,
    `verifier residual gaps: ${(((check && check.residual_gaps) || []).map((g) => (typeof g === 'string' ? g : g && (g.description || g.id)))).join('; ').slice(0, 600) || 'none'}`,
  ]
}

// Both mandatory reviews are runtime-provided map->reduce graphs: one critic map
// item per accepted task, a reduce_final that preserves every map finding.
const adversarial_findings = await adversarialReview(acceptedTaskIds, { evidenceFor: boundedEvidenceFor })
const uncovered_requirements = await coverageAudit(acceptedTaskIds, { evidenceFor: boundedEvidenceFor })

// Reviews run after every task is accepted, so act on them here. Blocked tasks are
// passed along too: they never appear in review findings, so they would otherwise
// be abandoned without one further bounded attempt.
const review_remediation = await remediateFindings([...(adversarial_findings || []), ...(uncovered_requirements || [])], {
  blockedTasks,
  taskFileFor: (id) => (byId(id) || {}).file,
  targetFilesFor: (id) => (byId(id) || {}).targetFiles,
})

// The mandatory final stage: the task set's frozen acceptance checks run
// against the repository as the run left it, and the host will not record the
// run complete while one fails.
phase('Acceptance')
const acceptance_gate = await acceptance({
  taskFileFor: (id) => (byId(id) || {}).file,
  targetFilesFor: (id) => (byId(id) || {}).targetFiles,
})

return {
  accepted: acceptedTaskIds,
  blocked: blockedTasks,
  adversarial_findings,
  uncovered_requirements,
  review_remediation,
  acceptance_gate,
  notes: `Two-task chain: ${acceptedTaskIds.length} accepted, ${blockedTasks.length} blocked after bounded remediation. AC-SYN-001's observer artifact .archon/proof/synthetic-observer-target.json is contractually outside every task's write ownership, so its exists-clause has no task-side producer and any such gap must come from the coverage audit rather than from task work.`,
}