// C6: a real corpus runner refusal retains its task bytes through the script loop.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js',
  'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js',
  'workflow_decompose_v1_seed.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
const [candidate, summary] = process.argv.slice(2);
assert.ok(JSON.parse(candidate).tasks.length > 0);
assert.match(summary, /invalid type/);
const ctx = withAuthorContext({ args: { gateMode: 'enforce' } });
vm.createContext(ctx);
vm.runInContext(source, ctx);
const options = [];
let freezes = 0;
ctx.authorCandidate({
  agent: async (_id, opts) => {
    options.push(opts);
    return options.length === 2
      ? { status: 'failed', invalid_answer: true, content: candidate, retainedLandings: true, summary }
      : { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true };
  },
  hostCommand: async () => ++freezes === 1
    ? { gateEnvelope: { policy_findings: [{ text: 'repair task contracts', remediation_scope: 'skeleton' }] } }
    : { publicationReceipt: {}, postcondition: { satisfied: true }, subjects: [], gateEnvelope: { policy_findings: [] } },
  pause: async () => { throw Error('unexpected pause'); }
}, { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Author records',
  retryScopes: new Set(['skeleton', 'candidate_artifact']) }).then(() => {
  assert.equal(options.length, 3);
  assert.equal(options[2].previousSkeleton, candidate);
  assert.ok(options[2].task.includes(summary));
  assert.ok(options[2].openFindings.includes('repair task contracts'));
  assert.equal(freezes, 2);
}).catch(error => { console.error(error); process.exitCode = 1; });
