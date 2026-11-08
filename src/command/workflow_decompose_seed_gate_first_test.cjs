const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const names = ['v1','v1_body','reply_blocks','v1_acceptance','v1_set_gate','v1_frozen', 'v1_prompt',
  'v1_progress','v1_failing_set','v1_seed','v1_context'];
const source = names.map(n => fs.readFileSync(`${__dirname}/workflow_decompose_${n}.js`, 'utf8')).join('\n');
const policy = { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Land tasks',
  retryScopes: new Set(['skeleton', 'candidate_artifact']) };
const candidate = JSON.stringify({schema_version:1,acceptance_digest:'',tasks:[]});
async function assertGateFirst(phaseSeed) {
  const ctx = withAuthorContext({args:{phaseSeed}});
  vm.createContext(ctx); vm.runInContext(source, ctx);
  ctx.applySeedOrdinals();
  const calls = [];
  const fresh = ['fresh gate finding'];
  await ctx.authorCandidate({
    agent: async (id, opts) => {
      calls.push(id);
      assert.equal(id, 'skeleton-author-6');
      assert.deepEqual(Array.from(opts.openFindings), fresh);
      assert.equal(opts.previousSkeleton, phaseSeed.subjects.skeleton.candidate);
      return {status:'accepted',stopReason:'end_turn',content:phaseSeed.subjects.skeleton.candidate,retainedLandings:true};
    },
    hostCommand: async (_, opts) => {
      calls.push('gate');
      assert.equal(opts.stdin, phaseSeed.subjects.skeleton.candidate);
      return calls.length === 1 ? {gateEnvelope:{policy_findings:fresh.map(text => ({text,remediation_scope:'skeleton'}))}}
        : {publicationReceipt:{},postcondition:{satisfied:true},subjects:[],gateEnvelope:{policy_findings:[]}};
    },
    pause: async () => { throw Error('unexpected pause'); }
  }, policy);
  assert.deepEqual(calls, ['gate','skeleton-author-6','gate']);
}
module.exports = {assertGateFirst};
test('unjudged accepted skeleton resumes at gate before author receives fresh findings', async () => {
  await assertGateFirst({subjects:{skeleton:{kind:'artifact',candidate,candidate_call:'skeleton-author-4',retained_landings:false,gate:null}},author_ordinals:{skeleton:5}});
});
test('artifact-refused bytes never become previousCandidate even with landing provenance', () => {
  const ctx = withAuthorContext({args:{}});
  vm.createContext(ctx); vm.runInContext(source, ctx);
  for (const retained_landings of [false, true]) {
    const result = ctx.seedArtifact({candidate, candidate_call:'skeleton-author-4', retained_landings,
      gate:{findings:[{text:'refused',remediation_scope:'candidate_artifact'}]}}, policy);
    assert.equal(result.previousCandidate, undefined);
    assert.equal(result.retainedLandings, false);
  }
});

test('candidate call ids do not imply retained landing provenance', () => {
  const ctx = withAuthorContext({args:{}});
  vm.createContext(ctx); vm.runInContext(source, ctx);
  for (const retained_landings of [false, true]) {
    const result = ctx.seedArtifact({candidate, candidate_call:'skeleton-author-4', retained_landings}, policy);
    assert.equal(result.retainedLandings, retained_landings);
  }
});
