const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {withAuthorContext, redactPublicTextFixture} = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const cases = fs.readFileSync(`${root}/workflow_parse_block_form_direction_cases.jsonl`, 'utf8')
  .trim().split('\n').map((line, index) => {
    try { return JSON.parse(line); }
    catch (error) { throw new Error(`fixture line ${index + 1}: ${error.message}`); }
  });
const context = {};
vm.createContext(withAuthorContext(context));
vm.runInContext(source, context);
context.__archonRedactPublicText = redactPublicTextFixture;
const parseRefusal = context.acceptanceParseRefusal;
const direction = 'the check is a multi-line script; return it in block form instead: set "command_block": true in the entry and put the script unescaped in exactly one fence labelled check after the JSON entry; then no escaping is needed';
// Only the block-form direction is shared: parser-generated refusal text differs legitimately between V8 and serde.
for (const [index, {reply, expectedBlockFormDirection}] of cases.entries()) {
  assert.equal(typeof reply, 'string', `fixture line ${index + 1} reply`);
  assert.equal(typeof expectedBlockFormDirection, 'string', `fixture line ${index + 1} expected block-form direction`);
  let error;
  try { JSON.parse(reply); }
  catch (caught) { error = caught; }
  assert.ok(error, `fixture line ${index + 1} must be malformed JSON`);
  const message = parseRefusal('A', reply, error);
  assert.equal(message.includes(direction) ? direction : '', expectedBlockFormDirection,
    `fixture line ${index + 1} produced: ${message}`);
}
console.log(`PASS shared command block-form direction cases (${cases.length} cases)`);
