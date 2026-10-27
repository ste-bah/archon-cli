// Issue 261 round 6: only a higher tier or a smaller distinct defect count progresses.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${scriptRoot}/${name}`, 'utf8')).join('\n');
const defect = (task, value = 'bad') => ({
  text: `candidate artifact was refused: task '${task}' file_name '${value}' is invalid`,
  subject: value, source_path: `/submitted/${value}`, remediation_scope: 'candidate_artifact',
  deterministic_defect: { provenance: 'host_validator', code: 'invalid_filename', subject: task, location: '' },
});
async function author(findings, resumed = [], mode = 'enforce', initialFeedback = []) {
  const ctx = { args: { gateMode: mode } };
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  const original = ctx.recordAttempt;
  const history = [];
  ctx.recordAttempt = (progress, ...params) => {
    const result = original(progress, ...params);
    history.push(JSON.parse(JSON.stringify(progress.history.at(-1))));
    return result;
  };
  let calls = 0;
  const pauses = [], rungs = [], prompts = [];
  const w = {
    checkpoint: async (_id, options) => { rungs.push({text:options.task, calls, history:history.slice()}); },
    agent: async (_id, options) => {
      prompts.push(options.task);
      if (++calls > 1000) throw new Error('renamingDefectsAdvancesWithoutProgress spin exceeded 1000 calls');
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
    retryScopes: new Set(['candidate_artifact']), initialFeedback }); } catch (e) { error = e.message; }
  return { calls, pauses, error, rungs, prompts };
}
const flags = out => out.rungs[0].history.map(e => e.progress);
const untilClean = findings => n => n <= 13 ? findings(n) : [];
function assertFirstWindow(out, expected = [true,false,false,false]) {
  assert.equal(out.error, undefined);
  assert.equal(out.calls, 14);
  assert.equal(out.pauses.length, 0);
  assert.equal(out.rungs[0].calls, 4);
  assert.deepEqual(flags(out), expected);
  assert.deepEqual(out.rungs.map(rung => /r\d/.exec(rung.text)[0]), ['r2','r3','r4','r2']);
}
async function renamingDefectsAdvancesWithoutProgress() {
  const out = await author(untilClean(n => [defect('TASK-X-001', `bad${n}`)]));
  assertFirstWindow(out);
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
async function rewordedFindingsAdvanceWithoutProgress() {
  const out = await author(untilClean(n => [{text:`parser is weak observation ${n}`,remediation_scope:'candidate_artifact'}]));
  assertFirstWindow(out);
}
async function oscillatingSubjectsAdvanceWithoutProgress() {
  const out = await author(untilClean(n => [defect(n % 2 ? 'TASK-X-001' : 'TASK-X-002')]));
  assertFirstWindow(out);
}
async function laterRungsPreserveTheBestMeasure() {
  const out = await author(untilClean(n => [defect(n > 4 && n % 2 ? 'TASK-X-002' : 'TASK-X-001')]));
  assertFirstWindow(out);
  assert.deepEqual(out.rungs[1].history.map(step=>step.progress), [true,false,false,false,false,false,false]);
}
// Round 7: a committed artifact with outstanding findings still pauses.
async function observeCommittedFindingsAdvanceTheLadder() {
  const out = await author(untilClean(() => [defect('TASK-X-001')]), [], 'observe');
  assertFirstWindow(out);
  assert.match(out.prompts[4], /file_name/);
}
async function observeSeededFindingsSurviveTheLadder() {
  const out = await author(untilClean(() => [defect('TASK-X-001')]), [], 'observe', ['seeded repair remains']);
  assertFirstWindow(out);
  assert(out.prompts.every(prompt=>prompt.includes('seeded repair remains')));
}
async function observeChangingFindingsAdvanceWithoutProgress() {
  const out = await author(untilClean(n => [defect(n % 2 ? 'TASK-X-001' : 'TASK-X-002')]), [], 'observe');
  assertFirstWindow(out);
}
async function duplicateIdentitiesDoNotEarnLadderCredit() {
  const out = await author(untilClean(n => n === 1 ? [defect('TASK-X-001'),defect('TASK-X-001','bad2')]
    : [defect('TASK-X-001',`bad${n}`)]));
  assertFirstWindow(out);
  assert.equal(out.rungs[0].history[0].findings, 1);
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
  for (const [name, test, guard] of [['renamingDefectsAdvancesWithoutProgress spin', renamingDefectsAdvancesWithoutProgress], ['70 to 0', seventy, true], ['shape to 70 defects is a higher tier', shapeThenSeventy, true],
    ['judged rewordedFindingsAdvanceWithoutProgress', rewordedFindingsAdvanceWithoutProgress], ['oscillatingSubjectsAdvanceWithoutProgress', oscillatingSubjectsAdvanceWithoutProgress], ['laterRungsPreserveTheBestMeasure best preserved', laterRungsPreserveTheBestMeasure],
    ['distinct identity count', duplicateIdentitiesDoNotEarnLadderCredit], ['observe committed findings advance the ladder', observeCommittedFindingsAdvanceTheLadder],
    ['observe seeded findings survive the ladder', observeSeededFindingsSurviveTheLadder], ['observe changing findings advance the ladder', observeChangingFindingsAdvanceWithoutProgress]]) {
    try { await test(); console.log(`PASS ${name}${guard ? ' (guard; validator completeness tested in Rust)' : ''}`); }
    catch (e) { failed++; console.error(`FAIL ${name}: ${e.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
