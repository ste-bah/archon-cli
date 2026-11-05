import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../crates/archon-workflow/src/v2/script/v3_primitives.js', import.meta.url), 'utf8');
// This prelude is concatenated with its remaining implementation by the host.
// Execute the complete batch primitive with a recording host boundary.
const prefix = source.slice(0, source.indexOf('  const phase ='));
function primitive() {
  const calls = [];
  const host = Object.fromEntries(['fanout', 'parallel'].map(method => [method,
    async (id, items, options) => { calls.push({method, id, items, options}); } ]));
  const {agents} = vm.runInNewContext(`${prefix}\nreturn {agents}; }\n__archonPrimitives(host)`, {host});
  return {agents, calls};
}

test('resource policy stays with the host across project tools and prose', async () => {
  for (const command of ['npm test', 'pytest a.py b.py', 'go test ./pkg', './gradlew test', 'custom-check --all', 'echo cargo test']) {
    const {agents, calls} = primitive();
    await agents([{prompt: 'Discuss cargo test output.', focusedTests: [command]}], {maxParallelism: 4});
    assert.equal(calls.length, 1);
    assert.equal(calls[0].method, 'parallel');
    assert.equal(calls[0].options.maxParallelism, 4);
    assert.equal(calls[0].items[0].focused_verification[0], command);
  }
});

test('write batches preserve explicit selectors and requested width', async () => {
  const {agents, calls} = primitive();
  await agents([{prompt: 'Implement the change.', targetFiles: ['src/example.py'],
    focusedTests: ['pytest tests/a.py::test_one tests/b.py::test_two']}], {write: true, maxParallelism: 3});
  assert.equal(calls[0].method, 'fanout');
  assert.equal(calls[0].options.maxParallelism, 3);
  assert.equal(calls[0].items[0].focused_verification[0], 'pytest tests/a.py::test_one tests/b.py::test_two');
});
