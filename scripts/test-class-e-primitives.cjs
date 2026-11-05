// Execute the real primitive prelude against a recording host; labels are arbitrary.
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const assert = require('node:assert/strict');
const dir = join(__dirname, '../crates/archon-workflow/src/v2/script');
const fragments = ['v3_primitives.js', 'v3_prim_budget.js', 'v3_prim_remediate_units.js',
  'v3_prim_remediate_rounds.js', 'v3_prim_remediate.js', 'v3_prim_complete.js', 'v3_prim_accept.js'];
const makePrimitives = new Function(fragments.map(f => readFileSync(join(dir, f), 'utf8')).join('\n')
  + '\nreturn __archonPrimitives;')();

async function recordWorkflow(labels) {
  const calls = [];
  const host = Object.fromEntries(['fanout', 'parallel', 'agent', 'checkpoint'].map(method =>
    [method, async (id, source, options) => { calls.push({ method, id, options: options || source }); return {}; }]));
  const api = makePrimitives(host);
  await api.agent('Produce the declared change', { label: labels[0], write: true,
    taskIds: ['TASK-EXAMPLE-001'], targetFiles: ['src/example.txt'] });
  await api.agent('Check the declared result', { label: labels[1], verify: true,
    taskIds: ['TASK-EXAMPLE-001'], focusedTests: ['check-result'] });
  await api.agent('Read the supplied evidence', { label: labels[2] });
  await api.agents([{ prompt: 'Check a result', label: labels[3], taskIds: ['TASK-EXAMPLE-001'] }], { verify: true });
  return calls;
}

test('renamed workflows declare identical behavior', async () => {
  const first = await recordWorkflow(['implement-task', 'verify-task', 'reader', 'sample']);
  const renamed = await recordWorkflow(['alpha', 'beta', 'verification-wave-misleading', 'delta']);
  assert.notDeepEqual(first.map(c => c.id), renamed.map(c => c.id));
  assert.deepEqual(first.map(c => c.options.metadata), renamed.map(c => c.options.metadata));
  assert.deepEqual(renamed.map(c => c.options.metadata.kind),
    ['implementation', 'focused_verification', 'other', 'focused_verification']);
  assert.equal(renamed[3].options.itemKind, 'focused_verification');
  assert.equal(renamed[0].options.metadata.source_graph, false);
});

test('generic read batches declare no verification role', async () => {
  const calls = [];
  const api = makePrimitives({ parallel: async (id, items, options) => { calls.push(options); return {}; } });
  await api.agents([{prompt: 'Inspect supplied context'}], {label: 'verification-wave-misleading'});
  assert.deepEqual(calls[0].metadata, {kind: 'other', task_scoped: false});
  assert.equal(calls[0].itemKind, undefined);
});
