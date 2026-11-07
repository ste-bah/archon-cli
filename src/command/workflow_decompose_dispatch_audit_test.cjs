// #630 C2/C4/C6: audit C section 2 classes B/C/I and section 3 D2/D4/D6.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const names = ['v1','v1_body','reply_blocks','v1_acceptance','v1_set_gate','v1_frozen',
  'v1_progress','v1_failing_set','v1_seed','v1_context'];
const source = names.map(n => fs.readFileSync(`${__dirname}/workflow_decompose_${n}.js`, 'utf8')).join('\n');
const candidate = JSON.stringify({ schema_version: 1, acceptance_digest: '', tasks: [
  { task_id: 'TASK-X-001', file_name: 'TASK-X-001.md', implements: ['REQ-1'] }
] });
const policy = { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Land tasks',
  retryScopes: new Set(['skeleton', 'candidate_artifact']) };
const clean = { publicationReceipt: {}, postcondition: { satisfied: true }, subjects: [], gateEnvelope: { policy_findings: [] } };
function context(args = {}) {
  const ctx = withAuthorContext({ args });
  vm.createContext(ctx); vm.runInContext(source, ctx); return ctx;
}
for (const incomplete of [false, true]) test(`C2 host findings exclude ${incomplete ? 'incomplete' : 'refusal'} feedback`, async () => {
  const ctx = context(); const options = []; let gates = 0;
  await ctx.authorCandidate({
    agent: async (_, opts) => {
      options.push(opts);
      if (options.length === 2) return incomplete
        ? { status: 'accepted', stopReason: 'max_tokens', content: candidate, retainedLandings: true }
        : { status: 'failed', invalid_answer: true, summary: 'bad receipt', content: candidate, retainedLandings: true };
      return { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true };
    },
    hostCommand: async () => ++gates === 1 ? { gateEnvelope: { policy_findings: [{ text: 'repair source', remediation_scope: 'skeleton' }] } } : clean,
    pause: async () => { throw Error('unexpected pause'); }
  }, policy);
  assert.deepEqual(Array.from(options[2].openFindings), ['repair source']);
  assert.match(options[2].task, incomplete ? /incomplete|truncated/i : /bad receipt/);
});
test('C4 pause evidence names invalid skeleton answers malformed', async () => {
  const ctx = context(); let pause;
  await assert.rejects(ctx.authorCandidate({
    agent: async () => ({ status: 'failed', invalid_answer: true, summary: 'reply must be exactly {"records_landed":0}' }),
    hostCommand: async () => { throw Error('unexpected gate'); },
    pause: async (_, opts) => { pause = opts; throw Error('paused'); }
  }, policy), /paused/);
  assert.match(JSON.stringify(pause), /malformed/);
  assert.doesNotMatch(JSON.stringify(pause), /"kind":"entries"/);
});
for (const seeded of [false, true]) test(`C6 zero-landed ${seeded ? 'seed-only' : 'first draft'} stays out of gate`, async () => {
  const ctx = context(); let gates = 0; const options = [];
  const content = seeded ? candidate : JSON.stringify({ schema_version: 1, acceptance_digest: '', tasks: [] });
  await assert.rejects(ctx.authorCandidate({
    agent: async (_, opts) => { options.push(opts); return { status: 'accepted', stopReason: 'end_turn', content }; },
    hostCommand: async () => { gates++; return clean; },
    pause: async () => { throw Error('paused'); }
  }, { ...policy, previousCandidate: seeded ? candidate : undefined }), /paused/);
  assert.equal(gates, 0);
  assert.match(options[1].task, /byte-changing landing/);
  assert.deepEqual(Array.from(options[1].openFindings), []);
});
test('C2 refusal before any gate leaves the host finding list empty', async () => {
  const ctx = context(); const options = [];
  await ctx.authorCandidate({
    agent: async (_, opts) => {
      options.push(opts);
      return options.length === 1 ? { status: 'failed', invalid_answer: true, summary: 'prose is not a receipt' }
        : { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true };
    }, hostCommand: async () => clean, pause: async () => { throw Error('unexpected pause'); }
  }, policy);
  assert.deepEqual(Array.from(options[1].openFindings), []);
  assert.match(options[1].task, /prose is not a receipt/);
});
test('C6 a changed landing reaches the gate and a durable seed can be rejudged', async () => {
  for (const resumed of [false, true]) {
    const ctx = context(resumed ? { phaseSeed: { subjects: { skeleton: { kind: 'artifact', candidate } } } } : {});
    let gates = 0;
    await ctx.authorCandidate({
      agent: async () => { assert.equal(resumed, false); return { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true }; },
      hostCommand: async opts => { gates++; return clean; }, pause: async () => { throw Error('unexpected pause'); }
    }, policy);
    assert.equal(gates, 1);
  }
});
// C1 audit D1: a recovered call's bytes survive the gate's artifact refusal too.
test('C1 artifact-refused recovered bytes never seed the next author', async () => {
  const ctx = context({ phaseSeed: { subjects: { skeleton: {
    kind: 'artifact', candidate, candidate_call: 'skeleton-author-4', retained_landings: true
  } } } });
  const options = []; let gates = 0;
  await ctx.authorCandidate({
    agent: async (_, opts) => { options.push(opts); return { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true }; },
    hostCommand: async () => ++gates === 1 ? { gateEnvelope: { policy_findings: [
      { text: 'candidate artifact was refused: repair shape', remediation_scope: 'candidate_artifact' }
    ] } } : clean,
    pause: async () => { throw Error('unexpected pause'); }
  }, policy);
  assert.equal(options[0].previousSkeleton, null);
  assert.deepEqual(Array.from(options[0].openFindings), []);
});
