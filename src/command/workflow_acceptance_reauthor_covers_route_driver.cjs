const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const ctx = withAuthorContext({args:{acceptanceCriteria:{'AC-F-001':'first output'},
  prdRequirementTexts: input.texts, authorMaxParallelism:1},
  __archonValidateAcceptanceEntry:() => '[]'});
vm.createContext(ctx);
for (const name of ['v1','v1_body','reply_blocks','v1_acceptance','v1_set_gate','v1_frozen','v1_progress','v1_context','v1_seed']) {
  vm.runInContext(fs.readFileSync(`${__dirname}/workflow_decompose_${name}.js`, 'utf8'), ctx);
}
ctx.requireDispatchable = async (_w, _label, task) => task;
ctx.catalogueText = ctx.priorText = () => '';
const prior = input.contract.acceptance[0];
const state = {entries:new Map([[prior.id, prior]]), retryIds:null};
state.retryIds = ctx.acceptanceRepairIds(input.findings, new Set([prior.id]), false,
  {entries:[prior], supplementary:[]}, state.entries);
assert.deepEqual([...state.retryIds], ['SUP-REQ-X-002']);
const calls = [];
ctx.authorAcceptanceEntries({agent:async (id, options) => {
  calls.push({id, task:options.task});
  return {status:'accepted', stopReason:'end_turn', content:JSON.stringify({
    ...prior, id:'SUP-REQ-X-002', criterion:input.texts['REQ-X-002'], covers:['REQ-X-002']})};
}}, 'author', 2, state).then(result => {
  assert.equal(result.status, 'accepted');
  assert.equal(calls.length, 1);
  assert.ok(calls[0].task.includes('SUP-REQ-X-002'));
  assert.ok(calls[0].task.includes(input.texts['REQ-X-002']));
  assert.deepEqual([...state.entries.get('SUP-REQ-X-002').covers], ['REQ-X-002']);
}).catch(error => { console.error(error); process.exitCode = 1; });
