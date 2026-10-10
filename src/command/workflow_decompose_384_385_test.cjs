// Issue 384 prompt rules and Issue 385 acceptance refusal/retry behavior.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const context = () => {
  const ctx = {args:{prdPath:'PRD.md', taskRoot:'tasks', repositoryRoot:'repo', projectRoot:'project',
    acceptanceCriteria:{A:'criterion'}, authorMaxParallelism:1, gateMode:'enforce'}};
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  return ctx;
};

function promptRules() {
  const ctx = context();
  const body = ctx.bodyPolicy({taskId:'TASK-X-001',fileName:'TASK-X-001.md'}, []).prompt();
  const acceptance = ctx.acceptanceAuthorPrompt();
  for (const rule of [
    'unquoted canonical TASK-<AREA>-<NNN>; its value must equal the TASK-<AREA>-<NNN> id in this filename, without the extension.',
    'Copy the frozen implements array exactly, preserving every value and its order.',
    'Every test file run by Focused Tests must be declared by an owning task',
    'Files Forbidden to Change entries must be literal paths, directories, basenames or globs',
    'Every MCP tool called by a Focused Tests command must appear in required_tools',
    'For each claimed PRD obligation, decide whether it is necessarily true',
  ]) assert.ok(body.includes(rule), `body prompt lacks rule: ${rule}`);
  assert.ok(acceptance.includes('fail on the pre-implementation baseline'), 'acceptance prompt lacks baseline proof rule');
  assert.ok(acceptance.includes('moves aside every data file it names by a path relative to its working directory'), 'acceptance prompt lacks already-true proof rule');
}

async function gateOutageReusesCandidate() {
  const ctx = context();
  let authors = 0, gates = 0;
  const candidate = '# authored task';
  const output = await ctx.authorCandidate({
    agent: async () => { authors++; return {status:'accepted',stopReason:'end_turn',content:candidate}; },
    hostCommand: async (_capability, request) => {
      gates++;
      assert.equal(request.stdin, candidate, 'the exact authored body reaches each gate attempt');
      return gates === 1 ? {gateEnvelope:{operational_error:{text:'temporary outage'}}} : {
        publicationReceipt:{call_id:'freeze'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}};
    },
    pause: async () => { throw new Error('unexpected pause'); },
  }, {phase:'body-TASK-X-001',prompt:()=>'',capability:'land-task-body',retryScopes:new Set(['body'])});
  assert.equal(authors, 1, 'an operational gate error must not call the author again');
  assert.equal(gates, 2);
  assert.equal(output.publicationReceipt.call_id, 'freeze');
}

async function wrongIdRefusalHasSpecificMeasuredProgress() {
  const ctx = context();
  let calls = 0; const pauses = [];
  await assert.rejects(ctx.authorCandidate({
    agent: async () => { calls++; return {status:'accepted',stopReason:'end_turn',content:'{"id":"B"}'}; },
    hostCommand: async () => { throw new Error('wrong id must never reach freeze'); },
    pause: async (_phase, evidence) => { pauses.push(evidence); throw new Error('paused'); },
  }, {phase:'acceptance',prompt:()=>'',author:ctx.authorAcceptanceEntries,
    capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])}), /paused/);
  assert.equal(calls, 4, 'the first refusal class advances; three repeats exhaust the window');
  const steps = Array.from(pauses[0].progress_history, step => [step.findings,step.progress]);
  assert.deepEqual(steps, [[1,true],[1,false],[1,false],[1,false]]);
  assert.match(pauses[0].last_findings.join(' '), /expected id A, got id "?B"?/);
}

async function wrongStopReasonIsSpecific() {
  const ctx = context();
  const out = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'max_tokens',content:'{"id":"A"}'
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  assert.match(out.summary, /expected stop reason end_turn, got max_tokens/);
}

async function missingContentIsSpecific() {
  const ctx = context();
  const out = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'end_turn'
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  assert.match(out.summary, /stop reason was end_turn but content was missing or empty/);
}

(async () => {
  for (const [name, test] of [['prompt rules',promptRules],['gate outage candidate reuse',gateOutageReusesCandidate],
    ['wrong id measured refusal',wrongIdRefusalHasSpecificMeasuredProgress],['specific stop reason',wrongStopReasonIsSpecific],
    ['specific missing content',missingContentIsSpecific]]) {
    await test(); console.log(`PASS ${name}`);
  }
})().catch(error => { console.error(`FAIL ${error.stack}`); process.exitCode = 1; });
