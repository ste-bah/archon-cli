// Issue 366: the acceptance author is told, before it writes a check, where
// the check can run (a copy at the freeze, a copy or the live checkout at an
// acceptance round), that it starts from its working directory and must name
// every path relative to it, never by the repository's or project's absolute
// path, and must change nothing outside its own temporary files; and the
// author step hands the shared entry validator the task set's roots, so a
// check naming one is refused in the same call. Generic: no PRD is named.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

const REPO = '/srv/example/code-repository';
const PROJECT = '/srv/example/project-root';
const RULE = "name every path relative to the check's working directory";

function context(validator) {
  const ctx = {args:{projectRoot:PROJECT, repositoryRoot:REPO, prdPath:`${PROJECT}/prd.md`, prdDigest:'d',
    taskRoot:`${PROJECT}/tasks`, gateMode:'enforce', authorMaxParallelism:1, acceptanceCriteria:{'AC-1':'one'}},
    __archonValidateAcceptanceEntry: validator || (() => '[]')};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  return ctx;
}

// One author round for `ids`; returns each entry's task text and the
// validator's calls.
async function round(ctx, owed = []) {
  for (const [id, requirement] of owed) {
    ctx.args.prdRequirementTexts = {...ctx.args.prdRequirementTexts, [requirement]:'owed text'};
    ctx.owedSupplementary().set(id, {requirement, text:'stale finding text'});
  }
  const tasks = new Map();
  const w = {agent: async (_, options) => {
    const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
    tasks.set(id, options.task);
    return {status:'accepted', stopReason:'end_turn', content:JSON.stringify({id, criterion:'',
      check:{kind:'command', command:'test -f out/result.json', cwd:'project_root'}})};
  }};
  const out = await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 1, {entries:new Map(), retryIds:null});
  return {tasks, out};
}

const cases = [];
const test = (name, fn) => cases.push([name, fn]);

test('the acceptance prompt states the execution model and the relative-path rule', () => {
  const ctx = context();
  assert.equal(typeof ctx.acceptanceAuthorPrompt, 'function', 'the acceptance prompt is one named function');
  const prompt = ctx.acceptanceAuthorPrompt();
  assert.match(prompt, /working directory/);
  assert.match(prompt, /project_root/);
  assert.match(prompt, /repo_root/);
  assert.match(prompt, /committed HEAD/);
  assert.ok(prompt.includes(RULE), 'the rule uses the freeze finding\'s own words');
  // The rule names both live roots as what a check must never write.
  const rule = prompt.split('\n').find(line => line.includes(RULE));
  assert.ok(rule.includes(REPO) && rule.includes(PROJECT), rule);
  assert.match(rule, /never write/i);
});

test('the prompt claims no isolation that may not exist', () => {
  const prompt = context().acceptanceAuthorPrompt();
  // Without an isolated execution policy an acceptance round runs checks in
  // the live checkout: the prompt must not promise otherwise.
  assert.ok(!/never runs a check in the live/i.test(prompt), 'the prompt promises a copy every time');
  assert.match(prompt, /in the live repository and project/);
  // The rule that holds at every site: change nothing outside its own
  // temporary files.
  assert.match(prompt, /must not change, delete or reset anything outside its own temporary files/);
  // Each site copies different project files, and places the two roots
  // differently: a check never reaches one root from the other.
  assert.match(prompt, /project files the host is configured to copy/);
  assert.match(prompt, /never reach one root from the other with \.\./);
});

test('the rule is generic: it names no PRD, project or language', () => {
  const rule = context().checkPathRule();
  for (const word of ['cargo', 'pytest', 'python', 'npm', 'REQ-', 'AC-']) {
    assert.ok(!rule.includes(word), `the rule names ${word}`);
  }
});

test('a supplementary entry is told the same rule', async () => {
  const ctx = context();
  const {tasks} = await round(ctx, [['SUP-REQ-9', 'REQ-9']]);
  const sup = tasks.get('SUP-REQ-9');
  assert.ok(sup, 'the owed supplementary check was authored');
  const own = sup.slice(sup.indexOf('Author ONLY entry SUP-REQ-9'));
  assert.ok(own.includes(RULE), 'the supplementary entry text itself repeats the rule');
  assert.ok(!/hermetic copy/.test(own), 'the supplementary text claims no isolation');
});

test('the author step gives the validator the task set it validates for', async () => {
  const calls = [];
  const ctx = context((...argv) => { calls.push(argv); return '[]'; });
  await round(ctx);
  assert.equal(calls.length, 1);
  const [id, serialized, roots] = calls[0];
  assert.equal(id, 'AC-1');
  assert.equal(JSON.parse(serialized).id, 'AC-1');
  // The roots the prompt names, and the task root the host reads the
  // freeze's own repository from: one source for both steps.
  assert.deepEqual(JSON.parse(roots), {repository:REPO, project:PROJECT, tasks:`${PROJECT}/tasks`});
});

test('a live-root refusal reaches the same entry\'s next call', async () => {
  const finding = `acceptance entry 'AC-1' was refused: check 'AC-1': it names the live root ${REPO} by its absolute path, so no hermetic copy can keep it off the live tree and the host never runs it; ${RULE}`;
  const ctx = context(() => JSON.stringify([{text:finding, deterministic_defect:{provenance:'host_validator',
    code:'live_root_path', subject:'entries/0/check', location:'command', stage:'contracts'}}]));
  const state = {entries:new Map(), retryIds:null};
  const tasks = [];
  const w = {agent: async (_, options) => {
    tasks.push(options.task);
    return {status:'accepted', stopReason:'end_turn', content:JSON.stringify({id:'AC-1', criterion:'',
      check:{kind:'command', command:`test -f ${REPO}/out`, cwd:'project_root'}})};
  }};
  const first = await ctx.authorAcceptanceEntries(w, 'author', 1, state);
  assert.equal(first.status, 'failed');
  assert.ok(first.findings.some(f => f.text === finding));
  await ctx.authorAcceptanceEntries(w, 'author', 2, state);
  assert.ok(tasks[1].includes(finding), 'the repair call shows the refusal verbatim');
});

(async () => {
  let failed = 0;
  for (const [name, fn] of cases) {
    try { await fn(); console.log(`ok - ${name}`); }
    catch (error) { failed += 1; console.log(`not ok - ${name}\n  ${error && error.stack}`); }
  }
  if (failed) { console.log(`${failed} of ${cases.length} failed`); process.exit(1); }
  console.log(`all ${cases.length} passed`);
})();
