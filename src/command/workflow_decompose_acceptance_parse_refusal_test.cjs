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
  assert.equal(blockRefusal('{}'),
    'acceptance entry A check.command_block must be boolean true');
  assert.equal(blockRefusal('{}', {kind:'command', command_block:true}),
    'acceptance entry A check.command_block is true but no check block is present');
  assert.equal(blockRefusal('```check\na\n```\n```check\nb\n```', {kind:'command', command_block:true}),
    'acceptance reply contains more than one check block');
  assert.equal(json.resolveCommandBlock({id:'A', check:{kind:'command', command:'run'}}, '```check\nrun\n```'),
    'check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block');
  assert.equal(blockRefusal('```check\n```', {kind:'command', command_block:true}), 'acceptance entry A check block is empty');
  assert.equal(blockRefusal('```check\nrun\n```', {kind:'command', command:'old', command_block:true}),
    'acceptance entry A returned both check.command and check.command_block');
  assert.equal(blockRefusal('```check X\nignored named fence\n```', {kind:'command', command:'run'}), null,
    'named fences are not check blocks');
  assert.equal(json.resolveCommandBlock({id:'A', check:{kind:'command', command:'echo "ok"'}}, ''), null,
    'inline commands remain valid');
  assert.equal(json.resolveCommandBlock({id:'A', check:{kind:'command', command:'run', command_block:false}}, ''), null,
    'false is an explicit absence of a command block');
  const liveReplies = JSON.parse(fs.readFileSync(`${root}/fixtures/decompose_live_reply_outcomes.json`, 'utf8'));
  assert.throws(() => json.extractJsonObject('{"id":"A"}\n```json\n{"id":"B"}\n```'), /reply contains more than one entry/);
  assert.throws(() => json.extractJsonObject('```json\n[{"id":"A"},{"id":"B"}]\n```'), /reply contains more than one entry/);
  assert.doesNotThrow(() => json.extractJsonObject('{"id":"A","check":{"command":"run","kind":"command"}}\n```json\n{"check":{"kind":"command","command":"run"},"id":"A"}\n```'));
  const candidateFixtures = JSON.parse(fs.readFileSync(`${root}/fixtures/decompose_json_candidate_outcomes.json`, 'utf8'));
  const mixedInvalid = candidateFixtures.find(fixture => fixture.id === 'mixed-invalid');
  assert.throws(() => json.extractJsonObject(mixedInvalid.reply), /reply contains more than one JSON object and one is not valid JSON:.*position/i);
  const identicalValid = candidateFixtures.find(fixture => fixture.id === 'identical-valid');
  assert.doesNotThrow(() => json.extractJsonObject(identicalValid.reply));
  const singleInvalid = candidateFixtures.find(fixture => fixture.id === 'single-invalid');
  const singleRefusal = await refusal(singleInvalid.reply);
  assert.match(singleRefusal.summary, /returned no complete entry: JSON parse error.*position/i);
  assert.match(singleRefusal.summary, /excerpt:/);
  for (const fixture of liveReplies) {
    let outcome;
    try {
      const candidate = JSON.parse(json.extractJsonObject(fixture.reply));
      const entry = Array.isArray(candidate) && candidate.length === 1 ? candidate[0] : candidate;
      outcome = json.resolveCommandBlock(entry, fixture.reply) || 'accepted';
    } catch (error) {
      outcome = error.message === 'reply contains more than one entry' ? error.message : 'malformed inline JSON escape';
    }
    if (fixture.id === 'SUP-REQ-AHDM-020') assert.equal(outcome, 'malformed inline JSON escape');
    if (fixture.id === 'SUP-REQ-AHDM-022') assert.equal(outcome, 'accepted', fixture.id);
    if (fixture.id === 'SUP-REQ-AHDM-021') assert.equal(outcome, 'reply contains more than one entry');
    if (fixture.id === 'SUP-REQ-BT-001') assert.equal(outcome, 'malformed inline JSON escape');
    if (fixture.id === 'live-prose-after-json-close' || fixture.id === 'live-close-and-open-same-line') {
      assert.equal(outcome, fixture.expected.slice('refuse: '.length), fixture.id);
    }
    if (fixture.id === 'corrected-fence-layout') assert.equal(outcome, 'accepted', fixture.id);
    if (fixture.id.startsWith('floor-command-block-')) {
      assert.equal(outcome, fixture.expected.slice('refuse: '.length), fixture.id);
    }
  }
  const resolved = await refusal('```check\nprintf ok\necho done\ntrue\n```\n{"id":"A","check":{"kind":"command","command_block":true}}');
  assert.equal(JSON.parse(resolved.content).entries[0].check.command, 'printf ok\necho done\ntrue');
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
  await exact('{"id":"A","check":{"kind":"command","command_block":true}}',
    'acceptance entry A check.command_block is true but no check block is present');
  await exact('```check\na\n```\n```check\nb\n```\n{"id":"A","check":{"kind":"command","command_block":true}}',
    'acceptance reply contains more than one check block');
  await exact('```check\nrun\n```\n{"id":"A","check":{"kind":"command","command":"run"}}',
    'check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block');
  await exact('```check\n```\n{"id":"A","check":{"kind":"command","command_block":true}}',
    'acceptance entry A check block is empty');
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
  assert.match(first.summary, /Use inline check\.command only for one short line without quotes or backslashes/);
  assert.ok(first.summary.includes('put the whole script in one block that opens with a line that is exactly ```check and set check.command_block true; never write a script inside a JSON string.'));
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
