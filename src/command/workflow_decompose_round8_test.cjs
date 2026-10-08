// Issue 261 round 8: the binding progress measure, and (Issue 288) observe's
// stall pause.
// Progress = a higher tier, OR a later first failing deterministic stage, OR
// at the same first failing stage a smaller TOTAL of distinct deterministic
// defects across all stages. Judge text is not measured.
// Runs the script from ARCHON_TEST_SCRIPT_ROOT (default: this directory).
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const det = (code, subject, stage, text = `${code} ${subject}`) => ({ text, subject,
  remediation_scope: 'candidate_artifact',
  deterministic_defect: { provenance: 'host_validator', code, subject, location: 'slot', stage } });
async function author(findings, { mode = 'enforce', outcome } = {}) {
  const ctx = { args: { gateMode: mode } };
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  let calls = 0;
  const pauses = [];
  const w = {
    agent: async () => {
      if (++calls > 200) throw new Error('spin');
      return { status: 'accepted', stopReason: 'end_turn', content: '{}' };
    },
    hostCommand: async () => ({ publicationReceipt: { call_id: `publication-${calls}` },
      postcondition: { satisfied: true }, ...(outcome ? outcome(calls) : {}),
      gateEnvelope: { policy_findings: findings(calls) } }),
    pause: async (_, evidence) => { pauses.push(evidence); throw new Error('paused'); },
  };
  let error, result;
  try {
    result = await ctx.authorCandidate(w, { phase: 'skeleton', prompt: () => 'author',
      retryScopes: new Set(['candidate_artifact']) });
  } catch (e) { error = e.message; }
  return { calls, pauses, error, result };
}
const range = (n, f) => Array.from({ length: Math.max(n, 0) }, (_, i) => f(i));

// Finding 2: a later-stage repair while an earlier defect remains.
async function laterStageRepairs() {
  const out = await author(n => n <= 4
    ? [det('missing_runnable_test', 'TASK-X-001', 'shape'), ...range(7 - n, i => det('invalid_verifier', `TASK-X-${i}`, 'contracts'))]
    : []);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 5);
}
// Finding 1: repository ownership defects 6 -> 5 -> 4 -> 3 carry identities.
async function ownership() {
  const out = await author(n => n <= 4 ? range(7 - n, i => det('unowned_repository_path', `src/file${i}.rs`, 'contracts',
    `src/file${i}.rs is named by the PRD but no task owns it`)) : []);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 5);
}
// Finding 3: per-element shape defects 4 -> 3 -> 2 -> 1, then structural.
async function shapes() {
  const out = await author(n => n <= 4
    ? range(5 - n, i => det('invalid_candidate_shape', `tasks/${i + n - 1}`, 'shape', `candidate artifact was refused: tasks/${i + n - 1} missing file_name`))
    : n <= 6 ? range(7 - n, i => det('invalid_filename', `TASK-X-00${i}`, 'structure', 'candidate artifact was refused: bad name')) : []);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 7);
}
// The total never rises past the best at the same first failing stage.
async function sameStageTotalMustFall() {
  const out = await author(n => [det('missing_runnable_test', 'TASK-X-001', 'shape'),
    ...range(n % 2 ? 3 : 4, i => det('invalid_verifier', `TASK-X-${i}`, 'contracts'))]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 4);
}
// Finding 5, under the Issue 288 rule: a stalled observe loop pauses
// (resumable) and returns no artifact. Its evidence names the LATEST round's
// findings, which match the live tree, not an earlier round with fewer.
async function observeStallPausesWithLatestFindings() {
  const out = await author(n => range(n === 2 ? 1 : 2, i => det('invalid_verifier', `TASK-X-${i}`, 'contracts')), {
    mode: 'observe',
    outcome: n => ({ subjects: n === 2 ? [{ taskId: 'TASK-X-001' }] : [{ taskId: 'TASK-X-001' }, { taskId: 'TASK-X-002' }] }),
  });
  assert.equal(out.error, 'paused', `observe must pause, got ${JSON.stringify(out.result)}`);
  assert.equal(out.result, undefined);
  assert.equal(out.calls, 5);
  assert.equal(out.pauses.length, 1);
  assert.equal(out.pauses[0].reason, 'no_progress');
  assert.deepEqual(Array.from(out.pauses[0].last_findings), ['invalid_verifier TASK-X-0', 'invalid_verifier TASK-X-1']);
}
(async () => {
  let failed = 0;
  for (const [name, test] of [['finding 2 later-stage repairs', laterStageRepairs],
    ['finding 1 ownership identities', ownership], ['finding 3 element shapes', shapes],
    ['same-stage total must fall', sameStageTotalMustFall], ['finding 5 observe stall pauses with the latest findings', observeStallPausesWithLatestFindings]]) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
