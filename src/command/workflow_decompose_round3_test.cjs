// Round 3 regressions: run directly with node, before and after the fixes.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${scriptRoot}/${name}`, 'utf8')).join('\n');
function context(criteria = { A: 'a' }) {
  const ctx = { args: { acceptanceCriteria: criteria, gateMode: 'enforce', authorMaxParallelism: 1 } };
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  return ctx;
}
const answer = content => ({ status: 'accepted', stopReason: 'end_turn', content });
const clean = () => ({ publicationReceipt: { call_id: 'freeze' }, postcondition: { satisfied: true },
  gateEnvelope: { policy_findings: [] } });
function policy(ctx) {
  return { phase: 'acceptance', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']),
    shadowScopes: new Set(), author: (w, prompt, round, state) =>
      ctx.authorAcceptanceEntries(w, prompt, round, state) };
}
// Round 4: a judge that rewords one defect makes every finding "new"; only a
// a smaller count is progress, so the loop pauses after three attempts without it.
async function novelty() {
  const ctx = context();
  let calls = 0, paused;
  const w = { agent: async () => {
      if (++calls > 500) throw new Error('spin: no pause after 500 calls');
      return answer('candidate');
    },
    hostCommand: async () => ({ ...clean(), gateEnvelope: { policy_findings:
      [{ text: `defect reworded ${calls}`, remediation_scope: 'candidate_artifact' }] } }),
    pause: async (_, evidence) => { paused = evidence; throw new Error('paused'); } };
  await assert.rejects(ctx.authorCandidate(w, { phase: 'body', prompt: () => 'author',
    retryScopes: new Set(['candidate_artifact']), shadowScopes: new Set() }), /paused/);
  assert.equal(calls, 4);
  assert.equal(paused.reason, 'no_progress');
}
async function partial(operational, replacing) {
  const ids = ['A', 'B', 'C', 'D'];
  const ctx = context(Object.fromEntries(ids.map(id => [id, id])));
  let gates = 0, paused;
  const completed = [];
  const w = { agent: async (callId) => {
    const [, id, ordinal] = /^acceptance-author-(\w+)-(\d+)$/.exec(callId);
    const round = Math.floor((Number(ordinal) - 1) / 3);
    if (replacing && round === 1) return answer(JSON.stringify({ id, version: 0 }));
    if (ids.indexOf(id) < round - (replacing ? 1 : 0)) {
      completed.push(id);
      return answer(JSON.stringify({ id, version: round }));
    }
    return operational ? { status: 'failed', summary: 'transport' } : answer('malformed');
  }, hostCommand: async () => replacing && ++gates === 1
    ? { ...clean(), gateEnvelope: { policy_findings: ids.map(id => ({
        text: `check '${id}': needs repair`, subject: id, remediation_scope: 'candidate_artifact' })) } }
  : clean(), pause: async (_, e) => {
    paused = e;
    throw new Error(replacing ? `unexpected pause with retained work ${completed}: ${JSON.stringify(e)}` : 'paused');
  } };
  if (replacing) {
    await ctx.authorCandidate(w, policy(ctx));
    assert.equal(gates, 2, 'a new malformed-reply refusal class advances once, then the repair reaches the gate');
  } else {
    await assert.rejects(ctx.authorCandidate(w, policy(ctx)), /paused/);
    assert.ok(!completed.includes('D'), 'three provider failures pause despite retained author work');
    assert.equal(paused.reason, 'operational_no_progress');
    assert.match(paused.recovery, /host provider/);
  }
}
async function unchangedReplacement() {
  const ctx = context({ A: 'a', B: 'b' });
  // A kept entry always carries its host-owned criterion (Issue 357).
  const state = { entries: new Map([['A', { id: 'A', version: 0, criterion: 'a' }]]), retryIds: null };
  const w = { agent: async id => id.includes('-A-')
    ? answer('{"version":0,"id":"A"}') : answer('malformed') };
  await ctx.authorAcceptanceEntries(w, 'author', 1, state);
  assert.equal(state.replaced, 0, 'reordered JSON is not a changed replacement');
  assert.equal(state.added, 0, 'an entry completed before is not a new one');
}
async function mixed() {
  const ctx = context();
  let calls = 0, paused;
  const w = { agent: async () => ++calls % 3 === 0 ? { status: 'failed', summary: 'transport' }
    : answer('malformed'), pause: async (_, e) => { paused = e; throw new Error('paused'); } };
  await assert.rejects(ctx.authorCandidate(w, policy(ctx)), /paused/);
  assert.equal(calls, 5, `a new malformed-reply refusal class advances before the independent windows stall: ${calls}`);
  assert.equal(paused.author_calls, 5, JSON.stringify(paused));
  assert.equal(paused.answered_attempts, 4);
}
(async () => {
  let failed = 0;
  for (const [name, test] of [ ['finding 1 novelty', novelty],
    ['finding 2A replacements', () => partial(false, true)],
    ['finding 2B provider window retains author work', () => partial(true, false)],
    ['finding 2 unchanged replacement', unchangedReplacement],
    ['finding 3 mixed failures', mixed] ]) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
