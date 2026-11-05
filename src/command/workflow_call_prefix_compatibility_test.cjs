const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const assert = require('node:assert/strict');

const dir = join(__dirname, '../../crates/archon-workflow/src/v2/script');
const fragments = ['v3_primitives.js', 'v3_prim_budget.js', 'v3_prim_remediate_units.js',
  'v3_prim_remediate_rounds.js', 'v3_prim_remediate.js', 'v3_prim_complete.js', 'v3_prim_accept.js'];
const makePrimitives = new Function(fragments.map(f => readFileSync(join(dir, f), 'utf8')).join('\n')
  + '\nreturn __archonPrimitives;')();

async function workflow(host) {
  const api = makePrimitives(host);
  await api.agent('Produce the declared change', { label: 'implement-task', write: true,
    taskIds: ['TASK-EXAMPLE-001'], targetFiles: ['src/example.txt'] });
  await api.agent('Check the declared result', { label: 'verify-task', verify: true,
    taskIds: ['TASK-EXAMPLE-001'], focusedTests: ['check-result'] });
  await api.agent('Read the supplied evidence', { label: 'reader' });
  await api.agents([{ prompt: 'Check a result', verify: true,
    focusedTests: ['check-result'], taskIds: ['TASK-EXAMPLE-001'] }], { verify: true });
}

test('primitive replay preserves call inputs and re-dispatches nothing', async () => {
  const records = new Map();
  const calls = [];
  let dispatches = 0;
  let resumed = false;
  const host = Object.fromEntries(['fanout', 'parallel', 'agent'].map(method =>
    [method, async (...args) => {
      // Durable JSON is the interface an old run retains, including options.
      const key = JSON.stringify({ method, args });
      if (records.has(key)) return records.get(key);
      assert.equal(resumed, false, 'resume changed a persisted call input');
      dispatches += 1;
      calls.push({ method, args });
      const result = { status: 'accepted', summary: args[0] };
      records.set(key, result);
      return result;
    }]));
  await workflow(host);
  assert.equal(dispatches, 4);
  assert.deepEqual(calls.map(c => c.args[0]),
    ['implement-task-1', 'verification-wave-verify-task-2', 'reader-3', 'agents-4']);
  for (const call of calls) {
    const options = call.args.at(-1);
    assert.equal(Object.hasOwn(options, 'metadata'), false);
  }
  dispatches = 0;
  resumed = true;
  await workflow(host);
  assert.equal(dispatches, 0);
});

test('read batches keep base options and gain no focused-verification guards', async () => {
  const calls = [];
  const api = makePrimitives({ parallel: async (...args) => { calls.push(args); return {}; } });
  await api.agents([{ prompt: 'Inspect the result', verify: true, focusedTests: ['check-result'] }],
    { label: 'inspection', verify: true, maxParallelism: 3 });
  const [id, items, options] = calls[0];
  assert.equal(id, 'inspection-1');
  assert.deepEqual(options, { tier: 'coder', maxParallelism: 3, task: 'Execute every item in this batch.' });
  assert.equal(Object.hasOwn(items[0], 'verification_requirements'), false);
  assert.equal(Object.hasOwn(items[0], 'baseline_test_rule'), false);
});
