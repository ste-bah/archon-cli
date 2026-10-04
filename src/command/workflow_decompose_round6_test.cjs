// Issue 261 round 6: only a higher tier or a smaller distinct defect count progresses.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js']
  .map(name => fs.readFileSync(`${scriptRoot}/${name}`, 'utf8')).join('\n');
const defect = (task, value = 'bad') => ({
  text: `candidate artifact was refused: task '${task}' file_name '${value}' is invalid`,
  subject: value, source_path: `/submitted/${value}`, remediation_scope: 'candidate_artifact',
  deterministic_defect: { provenance: 'host_validator', code: 'invalid_filename', subject: task, location: '' },
});
async function author(findings, resumed = [], mode = 'enforce') {
  const ctx = { args: { gateMode: mode } };
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  let calls = 0;
  const pauses = [];
  const w = {
    agent: async () => {
      if (++calls > 1000) throw new Error('rename spin exceeded 1000 calls');
      return { status: 'accepted', stopReason: 'end_turn', content: '{}' };
    },
    hostCommand: async () => ({ publicationReceipt: { call_id: 'r' }, postcondition: { satisfied: true },
      gateEnvelope: { policy_findings: findings(calls) } }),
    pause: async (id, evidence) => {
      pauses.push({ id, evidence });
      if (!resumed.includes(id)) throw new Error('paused');
    },
  };
  let error;
  try { await ctx.authorCandidate(w, { phase: 'skeleton', prompt: () => 'author',
    retryScopes: new Set(['candidate_artifact']) }); } catch (e) { error = e.message; }
  return { calls, pauses, error };
}
const flags = out => Array.from(out.pauses.at(-1).evidence.progress_history, e => e.progress);
async function rename() {
  const out = await author(n => [defect('TASK-X-001', `bad${n}`)]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 4);
  assert.deepEqual(flags(out), [true, false, false, false]);
}
async function seventy() {
  const out = await author(n => Array.from({ length: Math.max(71 - n, 0) }, (_, i) =>
    defect(`TASK-X-${String(i + n).padStart(3, '0')}`)));
  assert.equal(out.error, undefined);
  assert.equal(out.calls, 71);
  assert.equal(out.pauses.length, 0);
}
async function shapeThenSeventy() {
  const out = await author(n => n === 1 ? [{
    text: 'candidate artifact was refused: the JSON document does not match the required shape',
    remediation_scope: 'candidate_artifact',
    deterministic_defect: {provenance:'host_validator', code:'invalid_candidate_shape', subject:'skeleton', location:'candidate'},
  }] : Array.from({length: Math.max(72 - n, 0)}, (_, i) => defect(`TASK-X-${String(i + n).padStart(3, '0')}`)));
  assert.equal(out.error, undefined);
  assert.equal(out.calls, 72);
  assert.equal(out.pauses.length, 0);
}
async function rewording() {
  const out = await author(n => [{ text: `parser is weak observation ${n}`, remediation_scope: 'candidate_artifact' }]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 4);
}
async function oscillation() {
  const out = await author(n => [defect(n % 2 ? 'TASK-X-001' : 'TASK-X-002')]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 4);
  assert.deepEqual(flags(out), [true, false, false, false]);
}
async function resume() {
  const out = await author(n => [defect(n > 4 && n % 2 ? 'TASK-X-002' : 'TASK-X-001')], ['pause-skeleton-1']);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 7);
  assert.deepEqual(flags(out), [true, false, false, false, false, false, false]);
  assert.equal(out.pauses.length, 2);
}
// Round 7: observe falls back to the best committed artifact on a stall.
async function observeStall() {
  const out = await author(() => [defect('TASK-X-001')], [], 'observe');
  assert.equal(out.error, undefined);
  assert.equal(out.calls, 4);
  assert.equal(out.pauses.length, 0);
}
async function duplicates() {
  const out = await author(n => n === 1 ? [defect('TASK-X-001'), defect('TASK-X-001', 'bad2')]
    : [defect('TASK-X-001', `bad${n}`)]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 4);
  assert.equal(out.pauses[0].evidence.progress_history[0].findings, 1);
}
(async () => {
  if (process.argv[2]) {
    const envelopes = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
    const out = await author(n => envelopes[n - 1].policy_findings);
    assert.equal(out.error, undefined);
    assert.equal(out.calls, 71);
    assert.equal(out.pauses.length, 0);
    console.log('PASS real validator/envelope 70 to 0');
    return;
  }
  let failed = 0;
  for (const [name, test, guard] of [['rename spin', rename], ['70 to 0', seventy, true], ['shape to 70 defects is a higher tier', shapeThenSeventy, true],
    ['judged rewording', rewording], ['oscillation', oscillation], ['resume best preserved', resume],
    ['distinct identity count', duplicates], ['observe stall falls back to the best commit', observeStall]]) {
    try { await test(); console.log(`PASS ${name}${guard ? ' (guard; validator completeness tested in Rust)' : ''}`); }
    catch (e) { failed++; console.error(`FAIL ${name}: ${e.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
