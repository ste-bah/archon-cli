const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = [
  'workflow_decompose_v1.js',
  'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js',
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

async function evidenceWrongIdIsMeasuredByItsFailureClass() {
  const ctx = context();
  let calls = 0;
  const pauses = [];
  const before = `author_calls=${calls}, captured_pauses=${pauses.length}`;
  await assert.rejects(ctx.authorCandidate({
    agent: async () => { calls++; return { status: 'accepted', stopReason: 'end_turn', content: '{"id":"B"}' }; },
    hostCommand: async () => { throw new Error('refused candidate must not reach freeze'); },
    pause: async (_phase, evidence) => { pauses.push(evidence); throw new Error('pause captured'); },
  }, {
    phase: 'acceptance', prompt: () => '', author: ctx.authorAcceptanceEntries,
    capability: 'freeze-acceptance', retryScopes: new Set(['candidate_artifact']),
  }), /pause captured/);
  const refusal = pauses[0].last_findings.join(' ');
  const steps = Array.from(pauses[0].progress_history, step => step.progress);
  const after = `author_calls=${calls}, refusal=${JSON.stringify(refusal)}, progress=${JSON.stringify(steps)}`;
  console.log(`EVIDENCE #385 author-wrong-id BEFORE: ${before} AFTER: ${after}`);
  assert.match(refusal, /expected id A, got id "B"/);
  assert.deepEqual(steps, [true, false, false, false]);
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
  await evidenceWrongIdIsMeasuredByItsFailureClass();
  await evidenceMissingContentHasSpecificRefusal();
  await evidenceGateOutageReusesTheSameAuthoredCandidate();
  console.log('test result: ok. 3 evidence cases');
})().catch(error => { console.error(`FAIL ${error.stack}`); process.exitCode = 1; });
