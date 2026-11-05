// Run with node: supplementary identity always resolves through the host inventory.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const files = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js',
  'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js',
  'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js'];
const source = files.map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
const entry = id => ({id, criterion: 'current text', covers: [id.slice(4)],
  check: {kind: 'command', command: './verify', cwd: 'project_root'},
  gap_permitted: false, judgment: {verdict: 'accepted', counterexample: '', reason: '', host_call_id: ''}});
const policy = {phase: 'acceptance', author: true, retryScopes: new Set(['candidate_artifact'])};
function context(id) {
  const ctx = withAuthorContext({args: {prdRequirementTexts: {[id]: 'current text'}, acceptanceCriteria: {}},
    __archonValidateAcceptanceEntry: () => '[]'});
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  return ctx;
}
(async () => {
for (const id of ['REQ-X-001', 'FR-001', 'R1', 'REQ-1', 'US-12', 'item_1', 'Änderung.1', 'feature-login']) {
  const sup = `SUP-${id}`;
  let ctx = context(id);
  assert.equal(ctx.supplementaryRequirement(sup), id);
  assert.throws(() => ctx.supplementaryRequirement('SUP-UNKNOWN-99'), /missing source requirement UNKNOWN-99/);
  assert.throws(() => ctx.refreshFromCurrentContract({entries: new Map()},
    {current_acceptance_contract: {acceptance: [], supplementary: [entry('SUP-missing')]}},
    [], new Set(), null), /missing source requirement missing/);
  const text = `check '${sup}': PRD requirement ${id} is covered by no acceptance check; author supplementary check ${sup}: current text`;
  const finding = {text, subject: sup, remediation_scope: 'candidate_artifact'};
  assert.deepEqual([...ctx.acceptanceRepairIds([finding], new Set(), false)], [sup]);
  assert.equal(ctx.owedSupplementary().get(sup).text, 'current text');
  ctx = context(id);
  const typed = {...finding, text: 'opaque diagnostic', deterministic_defect: {code: 'uncovered_requirement', subject: sup}};
  assert.deepEqual([...ctx.acceptanceRepairIds([typed], new Set(), false)], [sup]);
  const candidate = JSON.stringify({entries: [], supplementary: [entry(sup)]});
  const state = {entries: new Map()};
  ctx.seedEntries({candidate, replies: [], invalid: {}, gates: []}, policy, state);
  assert.equal(state.entries.get(sup).criterion, 'current text');
  assert.equal(ctx.owedSupplementary().get(sup).requirement, id);
  assert.equal(state.retryIds.size, 0, 'unchanged seed is carried');
  ctx = context(id);
  const stale = {...entry(sup), criterion: 'old text'};
  const changed = {entries: new Map()};
  ctx.seedEntries({candidate: JSON.stringify({entries: [], supplementary: [stale]}), replies: [], invalid: {}, gates: []}, policy, changed);
  assert.deepEqual([...changed.retryIds], [sup], 'changed seed is reauthored');
  ctx = context(id);
  const published = {entries: new Map()};
  ctx.refreshFromCurrentContract(published, {current_acceptance_contract: {acceptance: [], supplementary: [entry(sup)]}}, [], new Set(), null);
  assert.equal(ctx.owedSupplementary().get(sup).requirement, id);
  assert.equal(published.retryIds.size, 0, 'published supplementary check is retained');
  ctx = context(id);
  let frozenState;
  ctx.authorAcceptanceEntries = async (_host, _prompt, _round, state) => {
    frozenState = state;
    return {};
  };
  ctx.authorCandidate = async (host, activePolicy) => {
    await activePolicy.author(host, 'repair', 1, {});
    return {};
  };
  const frozenFinding = {...finding, source_path: 'acceptance-contract.json'};
  await ctx.repairFrozenStage({}, {gateEnvelope: {report: JSON.stringify({acceptance: [], supplementary: [entry(sup)]})}}, [frozenFinding]);
  assert.equal(ctx.owedSupplementary().get(sup).requirement, id);
  assert.equal(frozenState.entries.get(sup).criterion, 'current text');
}
console.log('ok authored identity: inventory, repair, seed, drift, frozen repair, published refresh (8 styles)');
})().catch(error => { console.error(error); process.exitCode = 1; });
