// Parse refusals give the acceptance author a bounded repair clue and remain
// stable progress keys when the same malformed reply is repeated.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

async function refusal(reply) {
  const ctx = {args:{acceptanceCriteria:{A:'criterion'}, authorMaxParallelism:1}};
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  ctx.requireDispatchable = async (_w, _label, task) => task;
  ctx.catalogueText = () => '';
  ctx.priorText = () => '';
  const result = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted', stopReason:'end_turn', content:reply
  })}, 'author', 1, {entries:new Map(), retryIds:new Set(['A'])});
  return result.summary;
}

(async () => {
  const malformed = `{"id":"A","check":{"kind":"command","command":"${'x'.repeat(190)}${'\\q'}${'y'.repeat(190)}"}}`;
  const first = await refusal(malformed);
  const repeated = await refusal(malformed);
  const changed = await refusal(`{"id":"A","check":{"kind":"command","command":"${'x'.repeat(90)}${'\\q'}${'y'.repeat(290)}"}}`);
  assert.match(first, /JSON parse error.*Invalid \\escape|JSON parse error.*escape/i);
  assert.match(first, /(?:line\s+1\s+column\s+\d+|offset\s+\d+)/i);
  assert.match(first, /excerpt:/);
  assert.match(first, /check\.command is a JSON string value, so every double quote and backslash inside it is escaped\./);
  assert.equal(first, repeated, 'identical malformed replies have identical refusal text');
  assert.notEqual(ctxKey(first), ctxKey(changed), 'a changed parse position is a distinct progress key');
  const excerpt = first.match(/; excerpt: ("(?:\\.|[^"\\])*")/);
  assert.ok(excerpt, first);
  assert.ok(JSON.parse(excerpt[1]).length <= 160, 'excerpt is bounded to 160 characters');
  console.log('PASS acceptance parse refusal');
})().catch(error => { console.error(`FAIL acceptance parse refusal: ${error.stack}`); process.exitCode = 1; });

function ctxKey(text) {
  const ctx = {args:{}};
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  return ctx.findingKey(text);
}
