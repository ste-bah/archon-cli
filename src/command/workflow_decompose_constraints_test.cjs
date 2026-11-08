const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const names = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js",
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js'];
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = names.map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
function context(script = source) {
  const ctx = withAuthorContext({args:{prdPath:'/prd.md',taskRoot:'/tasks',repositoryRoot:'/repo',projectRoot:'/project',
    prdObligationIds:['REQ-X-001'],acceptanceCriteria:{'AC-X-001':'criterion'}}});
  vm.createContext(ctx); vm.runInContext(script, ctx); return ctx;
}
const subject = {taskId:'TASK-X-001',fileName:'TASK-X-001.md'};
function prompts(ctx) {
  return [ctx.skeletonPolicy([]).prompt(), ctx.bodyPolicy(subject, []).prompt(), ctx.acceptanceAuthorPrompt()];
}
function promptChecks() {
  const ctx = context();
  const absent = prompts(ctx);
  ctx.args.prdConstraintTexts = {};
  assert.deepEqual(prompts(ctx), absent, 'empty maps add no bytes, including no header or newline');
  ctx.args.prdConstraintTexts = {'QX-001':'restrict all work'};
  for (const prompt of prompts(ctx).slice(0, 2)) {
    assert.match(prompt, /QX-001/);
    assert.match(prompt, /every|each/);

  }
  const [skeleton, body, acceptance] = prompts(ctx);
  assert.equal(acceptance, absent[2], "constraints do not change acceptance author input");
  assert.match(skeleton, /all tasks if it restricts all work/);
  assert.match(body, /Read the host-listed PRD constraint texts; this task's work must keep every listed constraint its frozen implements names/);
  assert.doesNotMatch(body, /report that omission|name each applicable constraint id/);
  assert.match(body, /The host stamps implements, source_sections/);
  ctx.args.prdConstraintTexts = Object.fromEntries(Array.from({length:2000}, (_, n) => [`QX-${n}`, 'long restriction']));
  const lines = ctx.constraintContext();
  assert.ok(lines.join('\n').length < 1024);
  const path = lines[1].match(/at (\S+) \(host-written/)[1];
  assert.deepEqual(JSON.parse(ctx.__archonAuthorContext.files.get(path)), ctx.args.prdConstraintTexts);
  assert.deepEqual(ctx.constraintContext(), lines, 'replay uses the same context path');
  assert.notDeepEqual(prompts(ctx), absent, 'host map changes author input and its cache key');
}
async function resumedFrozenSetRepairsWithoutPause() {
  const ctx = context(); ctx.args.prdConstraintTexts = {'QX-001':'restrict work'};
  const oldEntry = {task_id:subject.taskId,file_name:subject.fileName,implements:['REQ-X-001'],depends_on:[],blocks:[],deliverable_contracts:[]};
  ctx.args.frozenChain = {acceptance:true,skeleton:true,subjects:[subject],bodies:[subject.fileName],tasks:[oldEntry]};
  const old = {subjects:[subject]};
  vm.runInContext('SKELETON_TASKS.set(__old, frozenChain().tasks)', Object.assign(ctx, {__old:old}));
  const chain = {outcome:old,subjects:[subject]}, bodies = new Map();
  let lint = 0, repairs = 0;
  const finding = {text:"PRD constraint 'QX-001' is stated as a PRD list item or table row that restricts work, but no task names it; add it to the implements of every task whose work it restricts (all tasks if it restricts all work)",
    subject:'QX-001',source_path:'/tasks/task-skeleton.json',remediation_scope:'skeleton',
    deterministic_defect:{provenance:'host_validator',code:'unowned_constraint',subject:'QX-001',location:'implements',stage:'contracts'}};
  const clean = () => ({publicationReceipt:{call_id:'ok'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}});
  const w = {hostCommand:async capability => {
    const result = clean();
    if (capability === 'task-set-lint' && ++lint === 1) result.gateEnvelope.policy_findings = [finding];
    return result;
  }, checkpoint:async()=>({}), pause:async()=>{throw new Error('must repair before any pause');}};
  // Keep host publication/body authoring mocked; exercise the real routing and set repair round.
  ctx.authorCandidate = async (_w, policy) => {
    repairs++; assert.equal(policy.phase, 'skeleton'); assert.ok(policy.initialFeedback.some(text=>text.includes('QX-001')));
    const outcome = {...clean(),subjects:[subject]};
    ctx.__next = outcome; ctx.__entry = {...oldEntry,implements:['REQ-X-001','QX-001']};
    vm.runInContext('SKELETON_TASKS.set(__next, new Map([[__entry.task_id, JSON.stringify(__entry)]]))', ctx);
    return outcome;
  };
  ctx.authorBodies = async () => {};
  await ctx.runSetGateLoop(w, chain, bodies);
  assert.equal(repairs, 1); assert.equal(lint, 2);
}
(async () => { promptChecks(); await resumedFrozenSetRepairsWithoutPause(); console.log('constraint prompts, bounded context, empty bytes and resumed set repair passed'); })().catch(error=>{console.error(error);process.exitCode=1;});
