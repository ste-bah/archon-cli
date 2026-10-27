// Parse refusals give the acceptance author a bounded repair clue and remain
// stable progress keys when the same malformed reply is repeated.
const { withAuthorContext, redactPublicTextFixture } = require('./workflow_decompose_context_stub.cjs');
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
  ctx.__archonRedactPublicText = redactPublicTextFixture;
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
  for (const [reply, expected] of [
    ['```check\r\nfirst\r\nsecond\r\n```', 'first\nsecond'],
    ['```check\nfirst\rsecond\r```', 'first\nsecond'],
    ['```check\r\nfirst\nsecond\rthird\n```', 'first\nsecond\nthird']
  ]) {
    const entry = {id:'A', check:{kind:'command', command_block:true}};
    assert.equal(json.resolveCommandBlock(entry, reply), null);
    assert.equal(entry.check.command, expected);
  }
  const entry = JSON.parse(json.extractJsonObject(fixture));
  assert.equal(entry.id, 'SUP-REQ-X', 'the check fence may be the first fence in the reply');
  assert.equal(json.resolveCommandBlock(entry, fixture), null);
  assert.equal(entry.check.command, 'printf \'%s\' "quote \\" slash \\\\ literal \\\\n"\necho ```',
    'the check command is the block bytes, except its one delimiter newline');
  assert.deepEqual(JSON.parse(JSON.stringify(entry.check)), {kind:'command', command:entry.check.command});
  const malformedFenceLayout = [
    '```json',
    '{"id":"A","check":{"kind":"command","command_block":true}}',
    '```check',
    'echo ok',
    'true'
  ].join('\n');
  assert.throws(() => json.extractJsonObject(malformedFenceLayout), (error) => {
    assert.equal(error.message,
      'the fence opened on line 1 is not closed; line 3 "```check" has an info string and cannot close it; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form');
    assert.ok(!error.message.includes('raw JSON object alone'));
    return true;
  });
  const markerFenceLayout = '{"id":"A","check":{"kind":"command","command_block":true}}<LEAK>```json\n{}\n```check\nrun';
  assert.throws(() => json.replyFenceBlocks(markerFenceLayout, ['<LEAK>']), (error) => {
    assert.equal(error.message,
      'the fence opened on line 1 is not closed; line 3 "```check" has an info string and cannot close it; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form');
    return true;
  });
  const liveDuplicateCheckOpeners = [
    '```json',
    '{"id":"A","check":{"kind":"command","command_block":true}}',
    '```check',
    '```check',
    '```'
  ].join('\n');
  assert.throws(() => json.replyFenceBlocks(liveDuplicateCheckOpeners), (error) => {
    assert.equal(error.message,
      'the fence opened on line 1 is not closed; line 3 "```check" has an info string and cannot close it; line 4 is a duplicate check fence opener; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form');
    return true;
  });
  const leakedMarkerInsideOpenFence = [
    '```json',
    'bad {"id":"A"} <LEAK> ```',
    '```check',
    'run',
    '```'
  ].join('\n');
  assert.throws(() => json.replyFenceBlocks(leakedMarkerInsideOpenFence, ['<LEAK>']), (error) => {
    assert.equal(error.message,
      'the fence opened on line 1 is not closed; line 3 "```check" has an info string and cannot close it; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form');
    return true;
  });
  const validFenceLayout = [
    '```json',
    '{"id":"A","check":{"kind":"command","command_block":true}}',
    '```',
    '```check',
    'echo ok',
    'true',
    '```'
  ].join('\n');
  const validFenceEntry = JSON.parse(json.extractJsonObject(validFenceLayout));
  assert.equal(json.resolveCommandBlock(validFenceEntry, validFenceLayout), null);
  assert.equal(validFenceEntry.check.command, 'echo ok\ntrue');
  assert.throws(() => json.replyFenceBlocks('```json\n{}'), /block opened at line 1 is never closed/);
  const blockRefusal = (body, check = {kind:'command', command_block:'X'}) => {
    const candidate = {id:'A', check};
    return json.resolveCommandBlock(candidate, body);
  };
  assert.equal(blockRefusal('{}'),
    'acceptance entry A check.command_block must be boolean true');
  assert.equal(blockRefusal('{}', {kind:'command', command_block:true}),
    "acceptance entry A check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: none");
  assert.equal(blockRefusal('```check js\nrun\n```', {kind:'command', command_block:true}),
    "acceptance entry A check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: 'check js'");
  assert.equal(blockRefusal('```check\na\n```\n```check\nb\n```', {kind:'command', command_block:true}),
    'acceptance reply contains more than one check block');
  assert.equal(json.resolveCommandBlock({id:'A', check:{kind:'command', command:'run'}}, '```check\nrun\n```'),
    'acceptance entry A mixes an inline command with a check fence and must use one form');
  const inferred = {id:'A', check:{kind:'command', cwd:'project_root'}};
  assert.equal(json.resolveCommandBlock(inferred, '```check\nrun\n```'), null);
  assert.deepEqual(JSON.parse(JSON.stringify(inferred.check)), {kind:'command', cwd:'project_root', command:'run'});
  assert.equal(blockRefusal('```check\n```', {kind:'command', command_block:true}), 'acceptance entry A check block is empty');
  assert.equal(blockRefusal('```check\nrun\n```', {kind:'command', command:'old', command_block:true}),
    'acceptance entry A returned both check.command and check.command_block — remove check.command from the JSON when you use the check block');
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
  const malformedEntryWithCheckFence = await refusal(
    '{"id":"A","check":{"kind":"command","command":"run",}}\n```check\nrun\n```');
  assert.match(malformedEntryWithCheckFence.summary, /JSON parse error.*line 1 column/i,
    malformedEntryWithCheckFence.summary);
  assert.ok(!malformedEntryWithCheckFence.summary.includes('return the raw JSON object alone'));
  for (const fixture of liveReplies) {
    if (fixture.id === 'SUP-REQ-AHDM-021' || fixture.id === 'live-prose-after-json-close' ||
        fixture.id === 'live-close-and-open-same-line') {
      assert.throws(() => json.extractJsonObject(fixture.reply), error => {
        assert.equal(error.message, fixture.expected.slice('refuse: '.length), fixture.id);
        return true;
      });
      continue;
    }
    let outcome;
    try {
      const candidate = JSON.parse(json.extractJsonObject(fixture.reply));
      const entry = Array.isArray(candidate) && candidate.length === 1 ? candidate[0] : candidate;
      outcome = json.resolveCommandBlock(entry, fixture.reply) || 'accepted';
    } catch (error) {
      outcome = error.message === 'reply contains more than one entry' ||
        /^the ```(?:json|check) block opened at line \d+ is never closed \(the reply may have been cut off\); close every block with a line that is exactly ```$/.test(error.message)
        ? error.message : 'malformed inline JSON escape';
    }
    if (fixture.id === 'SUP-REQ-AHDM-020') assert.equal(outcome, 'malformed inline JSON escape');
    if (fixture.id === 'SUP-REQ-AHDM-022') assert.equal(outcome, 'accepted', fixture.id);
    if (fixture.id === 'SUP-REQ-BT-001') assert.equal(outcome, 'malformed inline JSON escape');
    if (fixture.id === 'unicode-fence-label') {
      assert.equal(outcome, fixture.expected.slice('refuse: '.length), fixture.id);
    }
    if (fixture.id === 'corrected-fence-layout') assert.equal(outcome, 'accepted', fixture.id);
    if (fixture.id === 'single-check-block-implicit-command-block') assert.equal(outcome, 'accepted', fixture.id);
    if (fixture.id === 'false-command-block-with-check-fence' || fixture.id === 'command-and-command-block-with-check-fence' ||
        fixture.id === 'multiple-check-fences' || fixture.id === 'empty-check-fence' || fixture.id === 'check-fence-on-floor-kind') {
      assert.equal(outcome, fixture.expected.slice('refuse: '.length), fixture.id);
    }
    if (fixture.id.startsWith('floor-command-block-')) {
      assert.equal(outcome, fixture.expected.slice('refuse: '.length), fixture.id);
    }
  }
  const heredocReply = '{"id":"A","check":{"kind":"command","command_block":true}}\n```check\ncat <<\'EOF\'\n```js\n{"example": true}\nEOF\n```';
  const heredocEntry = JSON.parse(json.extractJsonObject(heredocReply));
  assert.equal(json.resolveCommandBlock(heredocEntry, heredocReply), null);
  assert.equal(heredocEntry.check.command, 'cat <<\'EOF\'\n```js\n{"example": true}\nEOF',
    'info-string fence text in a resolved check script is preserved byte for byte');
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
    "acceptance entry A check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: none");
  await exact('```check\na\n```\n```check\nb\n```\n{"id":"A","check":{"kind":"command","command_block":true}}',
    'acceptance reply contains more than one check block');
  await exact('```check\nrun\n```\n{"id":"A","check":{"kind":"command","command":"run"}}',
    'acceptance entry A mixes an inline command with a check fence and must use one form');
  await exact('```check\n```\n{"id":"A","check":{"kind":"command","command_block":true}}',
    'acceptance entry A check block is empty');
  const malformed = `{"id":"A","check":{"kind":"command","command":"${'x'.repeat(90)}${'\\q'}${'y'.repeat(290)}"}}`;
  const first = await refusal(malformed);
  const repeated = await refusal(malformed);
  const changed = await refusal(`{"id":"A","check":{"kind":"command","command":"${'x'.repeat(190)}${'\\q'}${'y'.repeat(190)}"}}`);
  const backward = await refusal(`{"id":"A","check":{"kind":"command","command":"${'x'.repeat(140)}${'\\q'}${'y'.repeat(240)}"}}`);
  const farther = await refusal(`{"id":"A","check":{"kind":"command","command":"${'x'.repeat(290)}${'\\q'}${'y'.repeat(90)}"}}`);
  const unknownParser = await refusal('opaque parser fixture');
  const unknownParserRepeat = await refusal('opaque parser fixture');
  const unknownStop = await refusal(JSON.stringify({id:'A'}), 'future_provider_stop');
  const unknownStopRepeat = await refusal(JSON.stringify({id:'A'}), 'another_future_stop');
  assert.match(first.summary, /JSON parse error.*Invalid \\escape|JSON parse error.*escape/i);
  assert.match(first.summary, /(?:line\s+1\s+column\s+\d+|offset\s+\d+)/i);
  assert.match(first.summary, /excerpt:/);
  assert.match(first.summary, /; excerpt: ".*<HERE>/);
  assert.ok(first.summary.includes('; a literal backslash inside a JSON string must be written as `\\\\`'), first.summary);
  assert.ok(first.summary.includes("Choose exactly one command form: for scripts, use the block form with kind and cwd, command_block true and no command field in the JSON check, followed by exactly one non-empty block whose fence info string is exactly 'check'; inline form puts one short line without quotes or backslashes in check.command with kind and cwd, and has no command_block or fence. Never write a script inside a JSON string."));
  const multilineScriptReply = String.raw`{"id":"A","check":{"kind":"command","command":"set -eu\nprintf x\necho \q"}}`;
  const multilineScriptRefusal = await refusal(multilineScriptReply);
  assert.ok(multilineScriptRefusal.summary.includes('the check is a multi-line script; return it in block form instead:'), multilineScriptRefusal.summary);
  assert.ok(multilineScriptRefusal.summary.includes('set "command_block": true in the entry'), multilineScriptRefusal.summary);
  assert.ok(multilineScriptRefusal.summary.includes('fence labelled check after the JSON entry; then no escaping is needed'), multilineScriptRefusal.summary);
  assert.ok(multilineScriptRefusal.summary.includes('; a literal backslash inside a JSON string must be written as `\\\\`'));
  const outsideCommandRefusal = await refusal(String.raw`{"id":"A","label":"bad\q","check":{"kind":"command","command":"echo ok"}}`);
  assert.ok(!outsideCommandRefusal.summary.includes('multi-line script'), outsideCommandRefusal.summary);
  const multilineOutsideError = await refusal(String.raw`{"id":"A","check":{"kind":"command","command":"first\nsecond"},"label":"bad\q"}`);
  assert.ok(multilineOutsideError.summary.includes('multi-line script'), multilineOutsideError.summary);
  assert.ok(multilineOutsideError.summary.includes('literal backslash inside a JSON string must be written as `\\\\`'));
  assert.equal(first.summary, repeated.summary, 'identical malformed replies have identical refusal text');
  assert.notEqual(first.malformedClass, changed.malformedClass, 'a changed parse position has a distinct diagnostic key');
  const excerpt = first.summary.match(/; excerpt: ("(?:\\.|[^"\\])*")/);
  assert.ok(excerpt, first);
  assert.ok(JSON.parse(excerpt[1]).length <= 160, 'excerpt is bounded to 160 characters');
  assert.match(first.malformedClass, /^parse-error:/);
  assert.equal(unknownParser.malformedClass, 'parse-error:other:location-unknown');
  assert.equal(unknownParserRepeat.malformedClass, unknownParser.malformedClass);
  assert.equal(unknownStop.malformedClass, 'stop-other');
  assert.equal(unknownStopRepeat.malformedClass, unknownStop.malformedClass);
  const progressContext = ctxForBlocks();
  const repairProgress = progressContext.newProgress();
  const progressFor = (malformedClass, call, entryId = 'A') => {
    progressContext.recordRepairs(repairProgress, call, [{entryId, findings:[], malformedClass}]);
    return repairProgress.history.at(-1).entries[0].progress;
  };
  const firstProgress = progressFor(first.malformedClass, 1);
  const repeatedProgress = progressFor(repeated.malformedClass, 2);
  const forwardProgress = progressFor(changed.malformedClass, 3);
  const backwardProgress = progressFor(backward.malformedClass, 4);
  const stalledAfterBackward = repairProgress.stalled;
  const sameFurthestProgress = progressFor(changed.malformedClass, 5);
  const stalledAfterSameFurthest = repairProgress.stalled;
  const fartherProgress = progressFor(farther.malformedClass, 6);
  const unknownProgress = progressFor(unknownParser.malformedClass, 7, 'B');
  const unknownRepeatProgress = progressFor(unknownParserRepeat.malformedClass, 8, 'B');
  const unknownKindProgress = progressFor('parse-error:unexpected-token:location-unknown', 9, 'B');
  const unknownAfterKnownProgress = progressFor(unknownParser.malformedClass, 10);
  const unknownChangedKindAfterKnownProgress = progressFor('parse-error:unexpected-token:location-unknown', 11);
  console.log(`EVIDENCE #383 same-position BEFORE: first progress=${firstProgress}`);
  console.log(`EVIDENCE #383 same-position AFTER: repeated progress=${repeatedProgress}`);
  console.log(`EVIDENCE #383 forward-position AFTER: progress=${forwardProgress}; key=${changed.malformedClass}`);
  console.log(`EVIDENCE #383 backward-position AFTER: progress=${backwardProgress}; stalled=${stalledAfterBackward}; key=${backward.malformedClass}`);
  console.log(`EVIDENCE #383 same-furthest AFTER: progress=${sameFurthestProgress}; stalled=${stalledAfterSameFurthest}`);
  console.log(`EVIDENCE #383 farther-position AFTER: progress=${fartherProgress}; key=${farther.malformedClass}`);
  console.log(`EVIDENCE #383 unknown-location BEFORE: first progress=${unknownProgress}`);
  console.log(`EVIDENCE #383 unknown-location AFTER: repeated progress=${unknownRepeatProgress}`);
  console.log(`EVIDENCE #383 unknown-kind-change AFTER: progress=${unknownKindProgress}`);
  console.log(`EVIDENCE #383 unknown-after-known-same-kind AFTER: progress=${unknownAfterKnownProgress}`);
  console.log(`EVIDENCE #383 unknown-after-known-new-kind AFTER: progress=${unknownChangedKindAfterKnownProgress}`);
  assert.equal(firstProgress, true);
  assert.equal(repeatedProgress, false);
  assert.equal(forwardProgress, true);
  assert.equal(backwardProgress, false);
  assert.equal(stalledAfterBackward, 1);
  assert.equal(sameFurthestProgress, false);
  assert.equal(stalledAfterSameFurthest, 2);
  assert.equal(fartherProgress, true);
  assert.equal(unknownProgress, true);
  assert.equal(unknownRepeatProgress, false);
  assert.equal(unknownKindProgress, true);
  assert.equal(unknownAfterKnownProgress, false);
  assert.equal(unknownChangedKindAfterKnownProgress, true);
  const unclosed = '```check\nrun\n{"id":"A"}';
  const fenceMessage = 'the ```check block opened at line 1 is never closed (the reply may have been cut off); close every block with a line that is exactly ```';
  assert.throws(() => json.extractJsonObject(unclosed), error => error.message === fenceMessage);
  assert.throws(() => json.resolveCommandBlock({id:'A', check:{kind:'command', command_block:true}}, unclosed),
    error => error.message === fenceMessage);
  const fenceRefusal = await refusal(unclosed);
  assert.equal(fenceRefusal.summary, `candidate artifact was refused: acceptance entry A: ${fenceMessage}`);
  assert.equal(fenceRefusal.malformedClass, 'unclosed-fence');
  const leakedMarker = '</think>';
  const liveReply = [
    '{"id":"A","check":{"kind":"command","command_block":true}}' + leakedMarker + '```check',
    ...Array.from({length:73}, (_, index) => `script line ${index + 1}`),
    '```'
  ].join('\n');
  assert.equal(liveReply.split('\n').length, 75);
  const livePrefix = '{"id":"A","check":{"kind":"command","command_block":true}}' + leakedMarker;
  assert.throws(() => json.extractJsonObject(liveReply), error =>
    error.message === `fence marker on line 1 follows text; a fence must start its own line; text before the marker: ${JSON.stringify(livePrefix)}`);
  const liveEntry = JSON.parse(json.extractJsonObject(liveReply, [leakedMarker]));
  assert.equal(json.resolveCommandBlock(liveEntry, liveReply, [leakedMarker]), null);
  assert.equal(liveEntry.check.command, Array.from({length:73}, (_, index) => `script line ${index + 1}`).join('\n'));
  assert.throws(() => json.extractJsonObject('```json\n{}\n```\n```'), error =>
    error.message === 'closing fence on line 4 has no opener; every closing fence must close an open block');
  assert.throws(() => json.extractJsonObject('{"id":"A"}\n```check\nscript'), /may have been cut off/);
  const trimmedLive = JSON.parse(fs.readFileSync(`${root}/fixtures/acceptance_author_unclosed_check_reply.json`, 'utf8')).content;
  assert.throws(() => json.extractJsonObject(trimmedLive), /the ```check block opened at line 2 is never closed/);
  const maxTokenRefusal = await refusal('{"id":"A"}', 'max_tokens');
  assert.match(maxTokenRefusal.summary, /cut off an incomplete reply of \d+ characters across \d+ lines/);
  assert.equal(maxTokenRefusal.malformedClass, 'stop-max-tokens');
  const missingStopRefusal = await refusal('{"id":"A"}', null);
  assert.match(missingStopRefusal.summary, /expected stop reason end_turn, got missing/);
  assert.equal(missingStopRefusal.malformedClass, 'stop-missing');
  // #383: the excerpt and the engine message are redacted with the host's
  // public-copy rule (events.rs sanitize_value): a secret-shaped word becomes
  // <redacted>, whole, even where the excerpt window cuts it.
  const secretKey = 'sk-ant-api03-' + 'A'.repeat(40);
  const nearSecret = `{"id":"A","check":{"kind":"command","command":"curl -H x-api-key ${secretKey} \\q here"}}`;
  const secretRefusal = await refusal(nearSecret);
  assert.ok(!secretRefusal.summary.includes(secretKey), secretRefusal.summary);
  assert.ok(!secretRefusal.summary.includes('A'.repeat(20)), 'no part of the key survives');
  assert.match(secretRefusal.summary, /excerpt: ".*x-api-key <redacted> /);
  const parseFn = ctxForBlocks().acceptanceParseRefusal;
  const at = (source, offset) => ({message: `Unexpected token at position ${offset}`});
  const unicodeWindowSource = 'é'.repeat(50) + String.raw`\q`;
  const unicodeWindowRefusal = parseFn('A', unicodeWindowSource, at(unicodeWindowSource, 50));
  const unicodeWindowExcerpt = JSON.parse(unicodeWindowRefusal.match(/; excerpt: ("(?:\\.|[^"\\])*")/)[1]);
  assert.equal(unicodeWindowExcerpt, 'é'.repeat(40) + '<HERE>\\q');
  const invalidEscape = String.raw`{"id":"A","check":{"kind":"command","command":"echo \q"}}`;
  const invalidEscapeError = (() => { try { JSON.parse(invalidEscape); } catch (error) { return error; } })();
  const invalidEscapeRefusal = parseFn('A', invalidEscape, invalidEscapeError);
  assert.match(invalidEscapeRefusal, /; excerpt: ".*<HERE>/);
  const invalidEscapeExcerpt = invalidEscapeRefusal.match(/; excerpt: ("(?:\\.|[^"\\])*")/);
  assert.ok(invalidEscapeExcerpt);
  assert.equal(JSON.parse(invalidEscapeExcerpt[1]), String.raw`eck":{"kind":"command","command":"echo \<HERE>q"}}`);
  assert.ok(invalidEscapeRefusal.includes('; a literal backslash inside a JSON string must be written as `\\\\`'));
  const splitSecret = String.raw`{"api-\qkey":"secret123"}`;
  const splitSecretError = (() => { try { JSON.parse(splitSecret); } catch (error) { return error; } })();
  const splitSecretRefusal = parseFn('A', splitSecret, splitSecretError);
  assert.ok(!splitSecretRefusal.includes('secret123'), splitSecretRefusal);
  assert.match(splitSecretRefusal, /at line 1 column \d+/);
  const splitSecretExcerpt = splitSecretRefusal.match(/; excerpt: ("(?:\\.|[^"\\])*")/);
  assert.ok(splitSecretExcerpt);
  assert.equal(JSON.parse(splitSecretExcerpt[1]), '{"api-key":"<redacted>"}');
  // A key that starts before the window and ends inside it.
  const straddle = 'x'.repeat(10) + ' ' + secretKey + ' ' + 'y'.repeat(200);
  const straddleText = parseFn('A', straddle, at(straddle, 11 + secretKey.length + 79 - 5));
  assert.ok(!straddleText.includes('A'.repeat(5)), straddleText);
  const straddleExcerpt = JSON.parse(straddleText.match(/; excerpt: ("(?:\\.|[^"\\])*")/)[1]);
  assert.match(straddleExcerpt, /^y{40}<HERE>y{40}$/);
  // `Bearer` before the window, its credential inside it.
  const bearer = 'z'.repeat(5) + ' Bearer ' + 'opaque-credential-value ' + 'w'.repeat(300);
  const bearerText = parseFn('A', bearer, at(bearer, 13 + 79 + 2));
  assert.ok(!bearerText.includes('opaque-credential'), bearerText);
  const bearerExcerpt = JSON.parse(bearerText.match(/; excerpt: ("(?:\\.|[^"\\])*")/)[1]);
  assert.match(bearerExcerpt, /^w{40}<HERE>w{40}$/);
  // key=value forms and an engine message that quotes the reply.
  const assigned = parseFn('A', '{"a":"run token=abc123 \\q"}', {message: 'Bad escape near "token=abc123 \\q" at position 22'});
  assert.ok(!assigned.includes('abc123'), assigned);
  assert.match(assigned, /JSON parse error at offset 22: Bad escape near <redacted> \\q" at position 22/);
  // An error at the very end: the window ends at the reply's end, no trailing ellipsis.
  const tail = 'p'.repeat(300) + ' {"x":';
  const tailText = parseFn('A', tail, at(tail, tail.length));
  const tailExcerpt = JSON.parse(tailText.match(/; excerpt: ("(?:\\.|[^"\\])*")/)[1]);
  assert.ok(tailExcerpt.endsWith('{"x":<HERE>'), tailExcerpt);
  assert.ok(Array.from(tailExcerpt).length <= 46, tailExcerpt);
  // A byte offset in a multi-byte reply is mapped to characters.
  const multi = 'é'.repeat(100) + ' MARK ' + 'é'.repeat(100);
  const multiText = parseFn('A', multi, {message: 'invalid at byte offset 201'});
  assert.match(multiText, /at byte offset 201/);
  assert.match(multiText, /MARK/);
  // A word that only looks like the rule's prefixes stays.
  const plain = parseFn('A', 'tokens=5 sk-short secret', at('', 3));
  assert.match(plain, /excerpt: "tok<HERE>ens=5 sk-short secret"/);
  console.log('PASS acceptance parse refusal');
})().catch(error => { console.error(`FAIL acceptance parse refusal: ${error.stack}`); process.exitCode = 1; });

function ctxForBlocks() {
  const ctx = {};
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  ctx.__archonRedactPublicText = redactPublicTextFixture;
  return ctx;
}
