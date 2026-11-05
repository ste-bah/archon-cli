const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;

test('body prompts assign repository observations to the host', () => {
  const ctx = vm.createContext({
    args: { repositoryRoot: '/repository', projectRoot: '/project', prdPath: '/input', taskRoot: '/tasks' },
    BODY_SHAPE: 'shape', constraintContext: () => [], sectionContext: () => [],
  });
  for (const name of ['workflow_decompose_v1_context.js', 'workflow_decompose_v1_body.js']) {
    vm.runInContext(fs.readFileSync(`${root}/${name}`, 'utf8'), ctx);
  }
  const prompt = vm.runInContext('bodyPolicy({taskId: "TASK-X-001", fileName: "TASK-X-001.md"}, []).prompt()', ctx);
  assert.match(prompt, /Do not write existence or line-count observations for paths; the host checks the repository itself\. Any existence or count you do state must be exact\./);
  assert.match(prompt, /Never claim a path exists or is absent unless you read it\./);
  assert.doesNotMatch(prompt, /exists \(N lines\)|last line number|no such observation|exactly one of these three observations/);
});
