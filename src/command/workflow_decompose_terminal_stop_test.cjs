// Issue 337: deliberate refusals use a typed host request, never error text.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

async function refusal(scope, setGate, frozen = false) {
  const requests = [];
  const stopped = new Error('host validated terminal stop');
  const ctx = { args: { gateMode: 'enforce' },
    __archonHost: async (method, payload) => {
      requests.push({ method, payload: JSON.parse(payload) });
      throw stopped;
    } };
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  const w = {
    agent: async () => ({status:'accepted',stopReason:'end_turn',content:'{}'}),
    hostCommand: async () => ({gateEnvelope:{policy_findings:[{
      remediation_scope:scope,text:'exact authoritative correction'
    }]}}),
  };
  await assert.rejects(frozen ? ctx.verifyFrozenStage(w, 'verify-frozen') : setGate ? ctx.runSetGate(w, 'set-gate')
    : ctx.authorCandidate(w, {phase:'acceptance',prompt:()=>'',retryScopes:new Set(['candidate_artifact'])}),
    error => error === stopped);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].method, 'terminalStop');
  assert.deepEqual(requests[0].payload, {schemaVersion:1,
    reason:`${frozen ? 'verify-frozen' : setGate ? 'set-gate' : 'acceptance'} stopped: ${scope === 'prd_input'
      ? '' : `scope '${scope}' is not actionable in this phase: `}exact authoritative correction`});
}

async function unhonoredRequestCannotReturnSuccess() {
  const ctx = { __archonHost: async () => '{}' };
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  await assert.rejects(async () => ctx.stopFixed('refusal'), /host returned without honoring the terminal stop/);
}

async function stopReasonIsBounded() {
  let payload;
  const stopped = new Error('host stopped');
  const ctx = { __archonHost: async (_, value) => { payload = JSON.parse(value); throw stopped; } };
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  await assert.rejects(async () => ctx.stopFixed('x'.repeat(20000)), error => error === stopped);
  assert.equal(payload.schemaVersion, 1);
  assert.equal(payload.reason.length, 4096);
}

(async () => {
  let failures = 0;
  for (const [name, test] of [
    ['author PRD refusal', () => refusal('prd_input', false)],
    ['set gate authoritative refusal', () => refusal('authoritative', true)],
    ['author unknown scope refusal', () => refusal('unknown_scope', false)],
    ['frozen stage PRD refusal', () => refusal('prd_input', false, true)],
    ['unhonored request cannot return success', unhonoredRequestCannotReturnSuccess],
    ['stop reason is bounded', stopReasonIsBounded],
  ]) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (error) { failures++; console.error(`FAIL ${name}: ${String(error.message).slice(0, 1000)}`); }
  }
  process.exitCode = failures ? 1 : 0;
})();
