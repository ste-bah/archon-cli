const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, 'workflow_decompose_reply_blocks.js'), 'utf8');
const context = {};
vm.createContext(context);
vm.runInContext(source + '\nglobalThis.api = { extractJsonObject, resolveCommandBlock, replyFenceBlocks };', context);
const { extractJsonObject, resolveCommandBlock, replyFenceBlocks } = context.api;
const markers = ['<LEAK_A>', '<LEAK_B>'];

assert.throws(
  () => replyFenceBlocks('```check\necho x', markers),
  /the reply may have been cut off/,
  'an unfinished check reply is identified as possibly cut off',
);

const reply = '<LEAK_A><LEAK_B>\r\n{"id":"A","check":{"kind":"command","command_block":true}}<LEAK_A><LEAK_B>```check\r\necho "<LEAK_A><LEAK_B>"\r\n```\r\n<LEAK_B>tail';
assert.equal(extractJsonObject(reply, markers), '{"id":"A","check":{"kind":"command","command_block":true}}');
const entry = { id: 'A', check: { kind: 'command', command_block: true } };
assert.equal(resolveCommandBlock(entry, reply, markers), null);
assert.equal(entry.check.command, 'echo "<LEAK_A><LEAK_B>"');

assert.throws(
  () => replyFenceBlocks('prefix ```check\nbody\n```', markers),
  /fence marker on line 1 follows text/,
  'a mid-line opener without the configured JSON boundary stays refused',
);

const jsonWithBraces = '{"id":"A","note":"a}b","nested":{"value":{"ok":true}},"check":{"kind":"command","command_block":true}}';
assert.equal(extractJsonObject(jsonWithBraces + '<LEAK_A>```check\necho ok\n```', markers), jsonWithBraces);
assert.throws(
  () => replyFenceBlocks('{"id":"A","nested":{"ok":true}} prose }<LEAK_A>```check\nbody\n```', markers),
  /fence marker on line 1 follows text/,
  'prose after a complete object cannot be skipped by searching for a later brace',
);
assert.throws(
  () => replyFenceBlocks(jsonWithBraces + '<LEAK_A>```check\nbody\n```', []),
  /fence marker on line 1 follows text/,
  'the live mid-line fence is refused without its configured marker',
);
