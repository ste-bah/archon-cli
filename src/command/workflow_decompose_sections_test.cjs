const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const names = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js",
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js'];
const source = names.map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
const subject = {taskId:'TASK-X-001',fileName:'TASK-X-001.md'};
const ctx = withAuthorContext({args:{prdPath:'/prd.md',taskRoot:'/tasks',repositoryRoot:'/repo',projectRoot:'/project',
  prdObligationIds:['REQ-X-001'],acceptanceCriteria:{'AC-X-001':'criterion'}}});
vm.createContext(ctx); vm.runInContext(source, ctx);
const absent = [ctx.skeletonPolicy([]).prompt(), ctx.bodyPolicy(subject, []).prompt()];
ctx.args.prdSections = {};
assert.deepEqual([ctx.skeletonPolicy([]).prompt(), ctx.bodyPolicy(subject, []).prompt()], absent);
ctx.args.prdSections = {
  '7': {title:'Widgets',text:'## 7. Widgets',requires_citation:false},
  '7.1': {title:'Small',text:'### 7.1 Small\nRequired widgets.',requires_citation:true},
  '8': {title:'Other',text:'## 8 Other\nUnrelated work.',requires_citation:true}
};
ctx.__entry = {task_id:subject.taskId,file_name:subject.fileName,source_sections:['7.1']};
vm.runInContext('recordSkeletonTasks({}, JSON.stringify({tasks:[__entry]}))', ctx);
const skeleton = ctx.skeletonPolicy([]).prompt();
assert.match(skeleton, /"7.1": Small/);
assert.doesNotMatch(skeleton, /"7": Widgets/);
assert.match(skeleton, /every section its work comes from/);
const body = ctx.bodyPolicy(subject, []).prompt();
assert.match(body, /task body must carry out what its frozen source_sections require for this task's work/);
const path = body.match(/Cited PRD section texts: Read (\S+)/)[1];
assert.deepEqual(JSON.parse(ctx.__archonAuthorContext.files.get(path)), {'7.1':ctx.args.prdSections['7.1']});
// Exact title keys use the same frozen carrier and bounded section context.
ctx.args.prdSections.Appendix = {title:'Appendix',text:'## Appendix\nMust keep appendix text.',requires_citation:true};
assert.match(ctx.skeletonPolicy([]).prompt(), /"Appendix": Appendix/);
assert.match(ctx.skeletonPolicy([]).prompt(), /by the key the host lists \(a section number, or the exact heading title of an unnumbered top-level section\)/);
ctx.__entry.source_sections = ['Appendix'];
vm.runInContext('recordSkeletonTasks({}, JSON.stringify({tasks:[__entry]}))', ctx);
const appendixBody = ctx.bodyPolicy(subject, []).prompt();
const appendixPath = appendixBody.match(/Cited PRD section texts: Read (\S+)/)[1];
assert.deepEqual(JSON.parse(ctx.__archonAuthorContext.files.get(appendixPath)), {Appendix:ctx.args.prdSections.Appendix});
// Preamble and combined duplicate/unnumbered text use the same frozen carrier.
ctx.args.prdSections['0'] = {title:'Example',text:'# Example\nRequired context.',requires_citation:true};
ctx.args.prdSections['7.1'] = {title:'Small / Shapes',
  text:'### 7.1 Small\nRequired widgets.\n#### Detail\nNever resize.\n### 7.1 Shapes\nKeep both.',requires_citation:true};
ctx.__entry.source_sections = ['0', '7.1'];
vm.runInContext('recordSkeletonTasks({}, JSON.stringify({tasks:[__entry]}))', ctx);
const combinedBody = ctx.bodyPolicy(subject, []).prompt();
const combinedPath = combinedBody.match(/Cited PRD section texts: Read (\S+)/)[1];
assert.deepEqual(JSON.parse(ctx.__archonAuthorContext.files.get(combinedPath)),
  {'0':ctx.args.prdSections['0'],'7.1':ctx.args.prdSections['7.1']});
const before = vm.runInContext('CURRENT_SKELETON_TASKS.get("TASK-X-001")',ctx);
ctx.__entry.source_sections = ['8'];
vm.runInContext('recordSkeletonTasks({}, JSON.stringify({tasks:[__entry]}))',ctx);
assert.notEqual(vm.runInContext('CURRENT_SKELETON_TASKS.get("TASK-X-001")',ctx),before,'citation changes invalidate frozen equality');
assert.match(ctx.bodyPolicy(subject, []).prompt(), /Frozen source_sections: \["8"\]/);
ctx.args.prdSections['8'].text = 'required '.repeat(50000);
const bounded = ctx.bodyPolicy(subject, []).prompt();
assert.ok(Buffer.byteLength(bounded) < 20000);
assert.equal(JSON.parse(ctx.__archonAuthorContext.files.get(bounded.match(/Cited PRD section texts: Read (\S+)/)[1]))['8'].text.length, 450000);
console.log('section prompts, frozen equality, cited context, empty context and bounds passed');

async function uncitedSectionRepairsWithoutPause() {
  const oldEntry = {...ctx.__entry, source_sections:[]};
  ctx.args.frozenChain = {acceptance:true,skeleton:true,subjects:[subject],bodies:[subject.fileName],tasks:[oldEntry]};
  const old = {subjects:[subject]};
  ctx.__old = old; ctx.__entry = oldEntry;
  vm.runInContext('SKELETON_TASKS.set(__old, frozenTaskEntries([__entry]))',ctx);
  const chain = {outcome:old,subjects:[subject]}, bodies = new Map();
  const finding = {text:"PRD section '7.1' (Small) is cited by no task's source_sections; add it to the source_sections of every task whose work it justifies",
    subject:'7.1',source_path:'/tasks/task-skeleton.json',remediation_scope:'skeleton',
    deterministic_defect:{provenance:'host_validator',code:'uncited_section',subject:'7.1',location:'source_sections',stage:'contracts'}};
  const clean = () => ({publicationReceipt:{call_id:'ok'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}});
  let lint = 0, repairs = 0;
  const w = {hostCommand:async capability => {
    const result = clean();
    if (capability === 'task-set-lint' && ++lint === 1) result.gateEnvelope.policy_findings = [finding];
    return result;
  },checkpoint:async()=>({}),pause:async()=>{throw new Error('section omission must repair before any pause');}};
  ctx.authorCandidate = async (_w,policy) => {
    repairs++; assert.equal(policy.phase,'skeleton'); assert.ok(policy.initialFeedback.some(text=>text.includes('7.1')));
    const next = {...clean(),subjects:[subject]}; ctx.__next = next;
    ctx.__entry = {...oldEntry,source_sections:['7.1']};
    vm.runInContext('recordSkeletonTasks(__next, JSON.stringify({tasks:[__entry]}))',ctx);
    return next;
  };
  ctx.authorBodies = async () => {};
  await ctx.runSetGateLoop(w,chain,bodies);
  assert.equal(repairs,1); assert.equal(lint,2);
  console.log('uncited section repaired in skeleton round without pause');
}
uncitedSectionRepairsWithoutPause().catch(error=>{console.error(error);process.exitCode=1;});
