// Parse refusals give the acceptance author a bounded repair clue and remain
// stable progress keys when the same malformed reply is repeated.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
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
  return {summary:result.summary, malformedClass:result.refusals && result.refusals[0] && result.refusals[0].malformedClass,
    content:result.content};
}

(async () => {
  const fixture = fs.readFileSync(`${root}/fixtures/decompose_command_block_reply.txt`, 'utf8');
  const json = ctxForBlocks();
  const entry = JSON.parse(json.extractJsonObject(fixture));
  assert.equal(entry.id, 'SUP-REQ-X', 'the check fence may be the first fence in the reply');
  assert.equal(json.resolveCommandBlock(entry, fixture), null);
  assert.equal(entry.check.command, 'printf \'%s\' "quote \\" slash \\\\ literal \\\\n"\necho ```',
    'the check command is the block bytes, except its one delimiter newline');
  assert.deepEqual(JSON.parse(JSON.stringify(entry.check)), {kind:'command', command:entry.check.command});
  const blockRefusal = (body, check = {kind:'command', command_block:'X'}) => {
    const candidate = {id:'A', check};
    return json.resolveCommandBlock(candidate, body);
  };
  assert.equal(blockRefusal('```check X\nrun\n```', {kind:'command', command:'old', command_block:'X'}),
    'acceptance entry A returned both check.command and check.command_block');
  assert.equal(blockRefusal('{}'), 'acceptance entry A command_block X has no matching check block');
  assert.equal(blockRefusal('```check X\na\n```\n```check X\nb\n```'), 'acceptance reply contains duplicate check block name X');
  assert.equal(blockRefusal('```check X\na\n```\n```check Y\nb\n```'), 'acceptance reply contains unreferenced check block Y');
  assert.equal(blockRefusal('```check X\n```'), 'acceptance entry A check block X is empty');
  assert.equal(json.resolveCommandBlock({id:'A', check:{kind:'command', command:'run'}}, '```check X\nrun\n```'),
    'acceptance reply contains unreferenced check block X');
  assert.equal(json.resolveCommandBlock({id:'A', check:{kind:'command', command:'echo "ok"'}}, ''), null,
    'inline commands remain valid');
  const resolved = await refusal('```check A\nprintf "%s" "quoted" \\\\ literal \\\\n\n```\n{"id":"A","check":{"kind":"command","command_block":"A"}}');
  assert.equal(JSON.parse(resolved.content).entries[0].check.command, 'printf "%s" "quoted" \\\\ literal \\\\n');
  const exact = async (reply, message) => {
    const result = await refusal(reply);
    assert.equal(result.summary, `candidate artifact was refused: acceptance entry A: ${message}`);
    assert.equal(result.malformedClass, 'command-block-refusal');
  };
  const nonEntries = JSON.parse(fs.readFileSync(`${root}/fixtures/decompose_non_entry_replies.json`, 'utf8'));
  for (const reply of nonEntries) {
    const result = await refusal(reply);
    assert.equal(result.summary,
      'candidate artifact was refused: acceptance entry A: reply did not contain a complete entry with id A', reply);
    assert.equal(result.malformedClass, 'missing-entry', reply);
  }
  await exact('{"id":"A","check":{"kind":"command","command":"old","command_block":"X"}}',
    'acceptance entry A returned both check.command and check.command_block');
  await exact('{"id":"A","check":{"kind":"command","command_block":"X"}}',
    'acceptance entry A command_block X has no matching check block');
  await exact('```check X\na\n```\n```check X\nb\n```\n{"id":"A","check":{"kind":"command","command_block":"X"}}',
    'acceptance reply contains duplicate check block name X');
  await exact('```check X\na\n```\n```check Y\nb\n```\n{"id":"A","check":{"kind":"command","command_block":"X"}}',
    'acceptance reply contains unreferenced check block Y');
  await exact('```check X\n```\n{"id":"A","check":{"kind":"command","command_block":"X"}}',
    'acceptance entry A check block X is empty');
  await exact('```check X\nrun\n```\n{"id":"A","check":{"kind":"command","command":"run"}}',
    'acceptance reply contains unreferenced check block X');
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
  assert.match(first.summary, /For inline check\.command, escape every double quote and backslash; use a fenced check block for multi-line or quote-heavy commands\./);
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

function ctxForBlocks() {
  const ctx = {};
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  return ctx;
}
