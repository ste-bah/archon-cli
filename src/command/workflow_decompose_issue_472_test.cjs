const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {withAuthorContext, redactPublicTextFixture} = require('./workflow_decompose_context_stub.cjs');

const root = __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const context = {};
vm.createContext(withAuthorContext(context));
vm.runInContext(source, context);
context.__archonRedactPublicText = redactPublicTextFixture;

const base = {id:'entry', check:{kind:'command', command:'run', cwd:'project_root'}};
const checkFence = '\n```check\nscript\n```';
for (const reply of [
  JSON.stringify(base),
  `\`\`\`json\n${JSON.stringify(base)}\n\`\`\``,
  JSON.stringify({...base, check:{kind:'command', command_block:true, cwd:'project_root'}}) + checkFence,
  `\`\`\`json\n${JSON.stringify({...base, check:{kind:'command', command_block:true, cwd:'project_root'}})}\n\`\`\`\n\`\`\`check\nscript\n\`\`\``
]) {
  assert.equal(JSON.parse(context.extractJsonObject(reply)).id, 'entry');
}

const mixed = context.resolveCommandBlock(structuredClone(base), JSON.stringify(base) + checkFence);
assert.equal(mixed, 'acceptance entry entry mixes an inline command with a check fence and must use one form');

for (const [name, expectedLocation, expectsBlockDirection] of [
  ['reply-one.txt', 'line 2 column 631', true],
  ['reply-two.txt', 'line 2 column 6520', true]
]) {
  const reply = fs.readFileSync(`${root}/../../tests/fixtures/reply-parser/${name}`, 'utf8');
  const json = '\n'.repeat(context.fencedEntryLinePrefix(reply)) + context.extractJsonObject(reply);
  let parseError;
  try { JSON.parse(json); } catch (error) { parseError = error; }
  assert.ok(parseError, `${name} must contain malformed JSON`);
  const message = context.acceptanceParseRefusal('entry', json, parseError);
  assert.ok(message.includes(expectedLocation), message);
  assert.ok(message.includes('excerpt:') && message.includes('<HERE>'), message);
  assert.ok(message.includes('invalid escape') || message.includes('Bad escaped character'), message);
  assert.equal(message.includes('the check is a multi-line script; return it in block form instead:'), expectsBlockDirection, message);
}

console.log('PASS issue 472 reply extraction, mixed-form refusal, and live fixtures');
