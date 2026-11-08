// Exercise the product prompt builder for both initial and patch authors.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const command = path.join(__dirname, '../../src/command');
const { withAuthorContext } = require(path.join(command, 'workflow_decompose_context_stub.cjs'));
const ctx = withAuthorContext({args:{prdPath:'prd.md',taskRoot:'tasks'}});
vm.createContext(ctx);
for (const name of ['v1', 'v1_body', 'v1_frozen', 'v1_prompt', 'v1_acceptance', 'v1_context', 'v1_set_gate']) {
  vm.runInContext(fs.readFileSync(path.join(command, `workflow_decompose_${name}.js`), 'utf8'), ctx);
}
process.stdout.write(JSON.stringify([
  ctx.skeletonPolicy([]).prompt(),
  ctx.skeletonPolicy([]).prompt('{"tasks":[]}')
]));
