const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const names = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js", 'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_context.js', 'workflow_decompose_v1_set_gate.js'];
const source = names.map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
function context() {
  const ctx = withAuthorContext({args:{prdPath:'prd.md', taskRoot:'tasks',
    prdConstraintTexts:{'QX-001':'bounded work'},
    prdSections:{'1':{title:'Closing',requires_citation:true,text:'Guidance.'}}}});
  vm.createContext(ctx); vm.runInContext(source,ctx); return ctx;
}
test('P5_9_constraint_ownership_prompt', () => {
  assert.ok(context().constraintContext().join('\n').includes('Every host-listed constraint id is in the implements of at least one task.'));
});
test('P5_10_only_host_listed_ids', () => {
  assert.ok(context().skeletonPolicy([]).prompt().includes('implements names only host-listed obligation or constraint ids'));
});
test('P5_16_section_remedy_prompt', () => {
  assert.ok(context().sectionContext().join('\n').includes('cite it in at least one task: every task whose work it justifies, or any one task when it justifies none (metadata, guidance or closing statements)'));
});
test('P5_25_one_landing_contract', () => {
  const prompt = context().skeletonPolicy([]).prompt();
  assert.ok(prompt.includes('Land each task with the landing tool; your final reply is exactly {"records_landed":N}, with N from the last landing receipt.'));
  assert.ok(!prompt.includes('Your entire reply must be the artifact itself'));
  for (const stale of ['one complete task-skeleton JSON artifact', 'The document must deserialize', 'acceptance_digest', 'landing summary']) assert.ok(!prompt.includes(stale), stale);
});
test('P5_26_constraint_placeholder', () => {
  assert.equal(vm.runInContext('JSON.parse(SKELETON_SHAPE).tasks[0].implements[0]',context()),'<obligation or constraint id the host lists>');
});
test('P5_27_reauthor_keep_list', () => {
  assert.ok(context().previousSkeletonContext('{"tasks":[]}').includes('keep every task, implements id, source_sections entry, deliverable and edge'));
});
test('offline_V5_resumed_patch_retains_unreached_bodies', async () => {
  const ctx = context();
  const subjects = [1,2].map(n=>({taskId:`TASK-X-00${n}`,fileName:`TASK-X-00${n}.md`}));
  const tasks = subjects.map(s=>({task_id:s.taskId,file_name:s.fileName,implements:['REQ-X-001'],source_sections:['1'],depends_on:[],blocks:[],deliverable_contracts:[]}));
  ctx.args.frozenChain={acceptance:true,skeleton:true,subjects,bodies:subjects.map(s=>s.fileName),tasks};
  const old={}, next={subjects}; ctx.__old=old;ctx.__next=next;
  vm.runInContext('SKELETON_TASKS.set(__old, frozenChain().tasks)',ctx);
  assert.deepEqual(JSON.parse(ctx.skeletonLandingSeed()).tasks.map(t=>t.task_id), tasks.map(t=>t.task_id));
  ctx.authorCandidate = async (_w,policy) => {
    assert.equal(policy.phase,'skeleton');
    const changed = tasks.map((t,n)=>n===0?{...t,implements:['REQ-X-001','QX-001']}:t);
    ctx.__changed=changed;
    vm.runInContext('SKELETON_TASKS.set(__next, frozenTaskEntries(__changed))',ctx);
    return next;
  };
  ctx.authorBodies = async (_w,reached)=>assert.deepEqual(JSON.parse(JSON.stringify(reached.map(row=>row[0]))),[subjects[0]]);
  await ctx.reauthorSkeleton({}, {outcome:old,subjects}, new Map(), [{text:'own QX-001',remediation_scope:'skeleton'}], [], 1);
});

const loopSource = ['workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_progress.js',
  'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_seed.js']
  .map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
const candidate = JSON.stringify({schema_version:1, acceptance_digest:'', tasks:[{
  task_id:'TASK-X-001', file_name:'TASK-X-001.md', implements:['QX-001'], source_sections:['1']}]});
const gate = findings => ({gateEnvelope:{policy_findings:findings}});
const clean = () => ({...gate([]), publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  subjects:[{taskId:'TASK-X-001',fileName:'TASK-X-001.md'}]});
async function seedRound(first, refusal, resume = false, previous) {
  const ctx = context(); vm.runInContext(loopSource, ctx);
  if (resume) ctx.args.phaseSeed = {subjects:{skeleton:{kind:'artifact',candidate:first,gate:{findings:refusal}}}};
  const options = [], inputs = [];
  await ctx.authorCandidate({
    agent:async (_id, opts) => {
      options.push(opts);
      if (options.length === 1 && !resume && (first === 'Landed tasks.' || first.includes('invalid.md')))
        return {status:'failed',invalid_answer:true,summary:refusal[0].text};
      return {status:'accepted',stopReason:'end_turn',retainedLandings:true,content:options.length === 1 && !resume ? first : candidate};
    },
    hostCommand:async (_id, opts) => { inputs.push(opts.stdin); return inputs.length === 1 && !resume && first === candidate ? gate(refusal) : clean(); },
    pause:async () => { throw new Error('unexpected pause'); }
  }, ctx.skeletonPolicy([], undefined, previous));
  return {ctx, options, inputs};
}
test('R1_P5_28_a_prose_reply_keeps_seed_and_reasks_parse_feedback', async () => {
  const feedback = 'candidate artifact was refused: expected JSON';
  for (const previous of [undefined, candidate]) {
    const {options} = await seedRound('Landed tasks.', [{text:feedback,remediation_scope:'candidate_artifact'}], false, previous);
    assert.equal(options.length,2);
    assert.equal(options[1].previousSkeleton, previous || null);
    assert.ok(options[1].task.includes(feedback));
    assert.equal(options[1].task.includes('The previous candidate is in'), !!previous);
  }
});
test('R1_P5_28_b_invalid_task_next_call_and_resume_start_agent', async () => {
  const bad = candidate.replaceAll('TASK-X-001','invalid');
  const findings = [{text:'invalid_task_id',remediation_scope:'candidate_artifact'}];
  for (const resume of [false,true]) {
    const {options} = await seedRound(bad, findings, resume);
    assert.equal(options.length, resume ? 1 : 2);
    assert.equal(options.at(-1).previousSkeleton, null);
    // Legacy durable candidates reach the defensive host seed reader; they do not prevent dispatch.
    assert.ok(options.at(-1).task.includes('invalid_task_id'));
  }
});
test('R1_P5_28_c_host_accepted_candidate_sets_seed', async () => {
  const {options} = await seedRound(candidate,[{text:'cite the missing section',remediation_scope:'skeleton'}]);
  assert.equal(options[1].previousSkeleton,candidate);
});
test('R5_frozen_reauthor_does_not_offer_remove', () => {
  const ctx = context(); ctx.args.frozenChain = {acceptance:true,skeleton:true,subjects:[],tasks:[],bodies:[]};
  assert.ok(!ctx.previousSkeletonContext(candidate).includes('remove:true'));
});

test('M2_P5_28_stale_resume_seed_does_not_drop_later_reopen_candidate', async () => {
  for (const findings of [[], [{text:'rebuild the stale candidate',remediation_scope:'candidate_artifact'}]]) {
    const ctx = context(); vm.runInContext(loopSource, ctx);
    ctx.args.phaseSeed = {subjects:{skeleton:{kind:'artifact',candidate}}};
    // The seeded call ignores the unjudged seed as an author setup input.
    assert.equal(ctx.skeletonPreviousCandidate(ctx.skeletonPolicy([]), candidate, {}), undefined);
    let hostCalls = 0;
    const options = [];
    const w = {
      agent:async (_id, opts) => { options.push(opts); return {status:'accepted',stopReason:'end_turn',retainedLandings:true,content:candidate}; },
      hostCommand:async () => ++hostCalls === 1 && findings.length ? gate(findings) : clean(),
      pause:async () => { throw new Error('unexpected pause'); }
    };
    // Carry and commit, with or without a candidate-artifact repair first.
    await ctx.authorCandidate(w, ctx.skeletonPolicy([]));
    assert.equal(options.length, findings.length ? 1 : 0);
    if (options.length) assert.equal(options[0].previousSkeleton, null);
    const finding = {text:'repair the set finding',remediation_scope:'skeleton'};
    await ctx.authorCandidate(w, ctx.skeletonPolicy([finding], 1, candidate));
    const reopened = options.at(-1);
    assert.equal(reopened.previousSkeleton, candidate);
    assert.ok(reopened.task.includes('The previous candidate is in'));
    assert.ok(reopened.task.includes('Compare the Landed subjects list at the end of this prompt with the previous candidate before landing changes and re-land every missing task with valid fields, preserving its work unless a current finding requires its removal. The host can drop invalid seeds. Only the subjects listed there have been retained. Re-land changed tasks.'));
    assert.ok(reopened.task.includes('Keep every frozen task_id and file_name.'));
  }
});

test('contract rule comes from the host and missing policy is explicit', () => {
  const ctx = context();
  ctx.args.deliverableContractRule = 'host contract policy';
  assert.ok(ctx.skeletonPolicy([]).prompt().includes('host contract policy'));
  delete ctx.args.deliverableContractRule;
  assert.throws(() => ctx.skeletonPolicy([]).prompt(), /host did not supply deliverableContractRule/);
});
