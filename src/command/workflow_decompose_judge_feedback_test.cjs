const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

async function judgeFeedbackReauthorsCandidate() {
  const ctx = { args: { gateMode: 'enforce' } };
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  const prompts = [], candidates = [], pauses = [];
  let authors = 0, gates = 0;
  const result = await ctx.authorCandidate({ checkpoint: async () => {}, 
    agent: async (_id, options) => {
      prompts.push(options.task);
      authors++;
      return { status: 'accepted', stopReason: 'end_turn', content: `candidate-${authors}` };
    },
    hostCommand: async () => {
      candidates.push(`candidate-${gates + 1}`);
      gates++;
      if (gates < 3) return { gateEnvelope: { operational_error: {
        kind: 'authored_judge_reply', text: 'judge response needs a complete verdict'
      } } };
      return { publicationReceipt: { call_id: 'published' }, postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] } };
    },
    pause: async (_subject, evidence) => { pauses.push(evidence); throw new Error('paused'); }
  }, { phase: 'skeleton', capability: 'freeze', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']) });
  assert.equal(authors, 3, 'each unusable judge response must trigger a new author call');
  assert.match(prompts[1], /judge response needs a complete verdict/);
  assert.match(prompts[2], /judge response needs a complete verdict/);
  assert.deepEqual(candidates, ['candidate-1', 'candidate-2', 'candidate-3']);
  assert.equal(pauses.length, 0);
  assert.ok(result.publicationReceipt);
}

judgeFeedbackReauthorsCandidate().then(() => console.log('PASS judge feedback reauthors candidate'), error => {
  console.error(`FAIL judge feedback reauthors candidate: ${error.message}`);
  process.exitCode = 1;
});
