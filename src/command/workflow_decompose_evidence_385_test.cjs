const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = [
  'workflow_decompose_v1.js',
  'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js',
  'workflow_decompose_v1_progress.js',
  'workflow_decompose_v1_context.js',
].map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

function context() {
  const value = { args: {
    prdPath: 'PRD.md', taskRoot: 'tasks', repositoryRoot: 'repo', projectRoot: 'project',
    acceptanceCriteria: { A: 'criterion' }, authorMaxParallelism: 1, gateMode: 'enforce',
  } };
  value.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(value));
  vm.runInContext(source, value);
  return value;
}

async function wrongIdEvidenceComesFromTheLadderTransition() {
  const ctx = context(); let calls = 0; const rungs = [], history = [];
  const original = ctx.recordRepairs;
  ctx.recordRepairs = (progress, ...params) => {
    const result = original(progress, ...params);
    history.push(JSON.parse(JSON.stringify(progress.history.at(-1)))); return result;
  };
  await ctx.authorCandidate({
    checkpoint: async (_id, options) => { rungs.push({text:options.task,calls,history:history.slice()}); },
    agent: async () => ({status:'accepted',stopReason:'end_turn',content: ++calls <= 13 ? '{"id":"B"}' : '{"id":"A"}'}),
    hostCommand: async () => ({publicationReceipt:{call_id:'published'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}}),
    pause: async () => {throw new Error('unexpected authored pause');}
  }, {phase:'acceptance',prompt:()=>'',author:ctx.authorAcceptanceEntries,capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])});
  assert.equal(calls,14);
  assert.equal(rungs[0].calls,4);
  assert.deepEqual(rungs[0].history.map(step=>[step.findings,step.progress]), [[1,true],[1,false],[1,false],[1,false]]);
  assert(rungs[0].history.every(step=>step.entries[0].malformed_class === 'wrong-id'));
}

async function evidenceMissingContentHasSpecificRefusal() {
  const ctx = context();
  const reply = { status: 'accepted', stopReason: 'end_turn' };
  const before = `stop_reason=${reply.stopReason}, content=${JSON.stringify(reply.content ?? null)}`;
  const result = await ctx.authorAcceptanceEntries({
    agent: async () => reply,
  }, 'acceptance author', 1, { entries: new Map(), retryIds: new Set(['A']) });
  const after = `summary=${JSON.stringify(result.summary)}`;
  console.log(`EVIDENCE #385 author-missing-content BEFORE: ${before} AFTER: ${after}`);
  assert.match(result.summary, /stop reason was end_turn but content was missing or empty/);
}

async function evidenceGateOutageReusesTheSameAuthoredCandidate() {
  const ctx = context();
  const bodies = [];
  let authorCalls = 0;
  const before = `author_calls=${authorCalls}, gate_calls=${bodies.length}`;
  const candidate = '# authored task';
  await ctx.authorCandidate({
    agent: async () => { authorCalls++; return { status: 'accepted', stopReason: 'end_turn', content: candidate }; },
    hostCommand: async (_capability, request) => {
      bodies.push(request.stdin);
      return bodies.length === 1
        ? { gateEnvelope: { operational_error: { text: 'temporary outage' } } }
        : { publicationReceipt: { call_id: 'published' }, postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] } };
    },
    pause: async () => { throw new Error('unexpected pause'); },
  }, {
    phase: 'body-TASK-X-001', prompt: () => '', capability: 'land-task-body',
    retryScopes: new Set(['body']),
  });
  const after = `author_calls=${authorCalls}, gate_calls=${bodies.length}, gate_bodies=${JSON.stringify(bodies)}`;
  console.log(`EVIDENCE #385 gate-outage-reuses-candidate BEFORE: ${before} AFTER: ${after}`);
  assert.equal(authorCalls, 1);
  assert.deepEqual(bodies, [candidate, candidate]);
}

(async () => {
  await wrongIdEvidenceComesFromTheLadderTransition();
  await evidenceMissingContentHasSpecificRefusal();
  await evidenceGateOutageReusesTheSameAuthoredCandidate();
  console.log('test result: ok. 3 evidence cases');
})().catch(error => { console.error(`FAIL ${error.stack}`); process.exitCode = 1; });
