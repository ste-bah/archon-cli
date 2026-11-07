const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js',
  'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const path = '/fixture/toolchain:/usr/bin:/bin';
const template = fs.readFileSync(`${root}/acceptance_scratch_policy.rs`, 'utf8')
  .split('\n').map(text => text.trim()).find(text => text.startsWith('"Checks run at'));
const line = JSON.parse(template).replace('{path}', path);
const entry = id => ({id, criterion:'criterion', covers:['REQ-X'],
  check:{kind:'command', cwd:'repo_root', command:'fixturetool extra'}, gap_permitted:false,
  judgment:{verdict:'accepted', counterexample:'', reason:'', host_call_id:''}});
function context(value) {
  const ctx = {args:{prdRequirementTexts:{'REQ-X':'requirement'}, acceptanceCriteria:{A:'criterion'}, authorMaxParallelism:1,
    acceptanceCheckSitePathLine:value}, __archonValidateAcceptanceEntry:() => '[]'};
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  return ctx;
}

test('site PATH is a fact in initial, repaired and supplementary author prompts only when known', async () => {
  for (const value of [line, undefined, null]) {
    const ctx = context(value), tasks = [];
    const w = {agent:async (_id, options) => {
      tasks.push(options.task);
      const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
      return {status:'accepted', stopReason:'end_turn', content:JSON.stringify(entry(id))};
    }};
    const state = {entries:new Map(), retryIds:null};
    await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 1, state);
    state.gateFindings = new Map([['A', ['repair this entry']]]);
    state.retryIds = new Set(['A']);
    await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 2, state);
    ctx.owedSupplementary().set('SUP-REQ-X', {requirement:'REQ-X', text:'requirement'});
    state.retryIds = new Set(['SUP-REQ-X']);
    await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 3, state);
    assert.equal(tasks.length, 3);
    for (const task of tasks) {
      if (typeof value === 'string') {
        const expected = value;
        assert.ok(task.split('\n').includes(expected));
        assert.ok(task.indexOf(expected) < task.indexOf('\nAuthor ONLY entry'));
      } else assert.doesNotMatch(task, /Checks run at the check site with PATH=/);
    }
  }
});

test('rewrite prompt shows the pre-round sibling version', async () => {
  const ctx = context(line), tasks = [];
  ctx.args.acceptanceCriteria = {B:'criterion b', C:'criterion c'};
  const w = {agent:async (_id, options) => {
    tasks.push(options.task);
    const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
    const repaired = entry(id);
    repaired.check.command = id === 'B' ? 'repairedtool run' : 'othertool run';
    return {status:'accepted', stopReason:'end_turn', content:JSON.stringify(repaired)};
  }};
  const state = {entries:new Map([['B', entry('B')], ['C', entry('C')]]), retryIds:new Set(['B', 'C']),
    gateFindings:new Map([['B', ["The host has no repair that makes the check's site provide it"]]])};
  await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 2, state);
  const prompt = tasks.find(text => text.includes('Author ONLY entry C:'));
  const record = prompt.split('\n').find(text => text.startsWith('- B sha256:'));
  assert.match(record, /covers=\[REQ-X\] command cwd=repo_root: fixturetool extra/);
  assert.doesNotMatch(record, /repairedtool run/);
  assert.doesNotMatch(record, /not a working example/);
});

test('live launch enriches site facts only after claiming a fresh task root', () => {
  const source = fs.readFileSync(`${root}/workflow_decompose.rs`, 'utf8');
  assert.ok(source.indexOf('create_claimed_run(') < source.indexOf('script_arguments_with_reply_markers('));
  assert.ok(source.indexOf('script_arguments_with_reply_markers(') < source.indexOf('WorkflowBundle::create_for_run('));
});

test('dry-run resume uses the live resume argument enrichment', () => {
  const source = fs.readFileSync(`${root}/workflow_decompose_resume_dry_run.rs`, 'utf8');
  assert.ok(source.includes('super::script_arguments_with_reply_markers(&script_arguments, &reply_leaked_markers)'));
});
