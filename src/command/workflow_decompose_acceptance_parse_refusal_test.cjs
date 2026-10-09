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

async function refusal(reply, stopReason = 'end_turn') {
  const ctx = {args:{acceptanceCriteria:{A:'criterion'}, authorMaxParallelism:1}};
  ctx.JSON = Object.assign(Object.create(JSON), {parse(text, ...args) {
    if (reply === 'opaque parser fixture' && text === reply) throw new SyntaxError('provider parser diagnostic');
    return JSON.parse(text, ...args);
  }});
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  ctx.requireDispatchable = async (_w, _label, task) => task;
  ctx.catalogueText = () => '';
  ctx.priorText = () => '';
  const result = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted', stopReason, content:reply
  })}, 'author', 1, {entries:new Map(), retryIds:new Set(['A'])});
  return {summary:result.summary, malformedClass:result.refusals[0].malformedClass};
}

(async () => {
  const malformed = `{"id":"A","check":{"kind":"command","command":"${'x'.repeat(190)}${'\\q'}${'y'.repeat(190)}"}}`;
  const first = await refusal(malformed);
  const repeated = await refusal(malformed);
  const changed = await refusal(`{"id":"A","check":{"kind":"command","command":"${'x'.repeat(90)}${'\\q'}${'y'.repeat(290)}"}}`);
  const unknownParser = await refusal('opaque parser fixture');
  const unknownParserRepeat = await refusal('opaque parser fixture');
  const unknownStop = await refusal(JSON.stringify({id:'A'}), 'future_provider_stop');
  const unknownStopRepeat = await refusal(JSON.stringify({id:'A'}), 'another_future_stop');
  assert.match(first.summary, /JSON parse error.*Invalid \\escape|JSON parse error.*escape/i);
  assert.match(first.summary, /(?:line\s+1\s+column\s+\d+|offset\s+\d+)/i);
  assert.match(first.summary, /excerpt:/);
  assert.match(first.summary, /check\.command is a JSON string value, so every double quote and backslash inside it is escaped\./);
  assert.equal(first.summary, repeated.summary, 'identical malformed replies have identical refusal text');
  assert.equal(first.malformedClass, changed.malformedClass, 'a changed parse position is the same progress class');
  const excerpt = first.summary.match(/; excerpt: ("(?:\\.|[^"\\])*")/);
  assert.ok(excerpt, first);
  assert.ok(JSON.parse(excerpt[1]).length <= 160, 'excerpt is bounded to 160 characters');
  assert.match(first.malformedClass, /^parse-error:/);
  assert.equal(unknownParser.malformedClass, 'parse-error:other');
  assert.equal(unknownParserRepeat.malformedClass, unknownParser.malformedClass);
  assert.equal(unknownStop.malformedClass, 'stop-other');
  assert.equal(unknownStopRepeat.malformedClass, unknownStop.malformedClass);
  console.log('PASS acceptance parse refusal');
})().catch(error => { console.error(`FAIL acceptance parse refusal: ${error.stack}`); process.exitCode = 1; });
