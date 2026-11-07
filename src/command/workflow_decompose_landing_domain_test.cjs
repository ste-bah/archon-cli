const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const names = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js',
  'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js',
  'workflow_decompose_v1_seed.js', 'workflow_decompose_v1_context.js'];
const source = names.map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
const directive = 'The frozen predecessor was repaired; rebind the skeleton while retaining every unaffected task entry.';
const task = { task_id:'TASK-X-001', file_name:'TASK-X-001.md', implements:['REQ-1'] };
for (const [name, seeded, findings, rebind] of [
  ['first draft', false, [], false], ['fresh retry', false, [], false],
  ['repair', true, ['repair task'], false], ['rebind', true, [], true],
  ['seeded', true, [], false], ['rebind with findings', true, ['repair task'], true]
]) test(`raw skeleton domain: ${name}`, async () => {
  const ctx = withAuthorContext({args:{gateMode:'enforce'}});
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  const previous = seeded ? JSON.stringify({tasks:[task]}) : undefined;
  const policy = ctx.skeletonPolicy(findings, undefined, previous, rebind ? directive : '');
  if (rebind) assert.equal(ctx.seedWouldReplaceLiveFeedback(policy), true, 'rebind must dispatch instead of replaying an older seed');
  const progressInputs = [];
  const progress = ctx.newProgress;
  ctx.newProgress = input => { progressInputs.push([...input]); return progress(input); };
  const options = [], frozen = [];
  const zero = JSON.stringify({tasks:seeded ? [task] : []});
  const changed = JSON.stringify({tasks:[{...task, source_sections:['2']}]});
  await ctx.authorCandidate({
    agent:async (_id, opts) => {
      options.push(opts);
      assert(options.length <= 2, 'bounded calls');
      return {status:'accepted', stopReason:'end_turn', content:options.length === 1 ? zero : changed,
        retainedLandings:options.length > 1};
    },
    hostCommand:async (_id, opts) => {
      frozen.push(opts.stdin);
      return {publicationReceipt:{}, postcondition:{satisfied:true}, subjects:[], gateEnvelope:{policy_findings:[]}};
    },
    pause:async () => { throw Error('unexpected pause'); }
  }, policy);
  const completes = seeded && findings.length === 0;
  assert.equal(options.length, completes ? 1 : 2);
  assert.deepEqual(frozen, [completes ? zero : changed]);
  assert.deepEqual(progressInputs, [findings]);
  for (const opts of options) {
    assert.deepEqual([...opts.openFindings], findings);
    assert.equal(opts.task.includes(directive), rebind);
    if (rebind) assert.equal(opts.task.split(directive).length - 1, 1, 'directive is an instruction only');
  }
  const landed = {status:'accepted', stopReason:'end_turn', retainedLandings:true, content:changed};
  assert.equal(ctx.refuseUnchangedSkeleton(landed, null, findings), landed, 'a landing completes every row');
});

test('acceptance repair routes rebind context separately from findings at both entry points', async () => {
  for (const viaSkeleton of [false, true]) {
    const ctx = withAuthorContext({args:{gateMode:'enforce'}});
    vm.createContext(ctx);
    vm.runInContext(source, ctx);
    const acceptance = {}, skeleton = {subjects:[{taskId:'TASK-X-001',fileName:'TASK-X-001.md'}]};
    vm.runInContext('globalThis.repairs = FROZEN_REPAIRS', ctx);
    ctx.repairs.set(acceptance, {owner:'acceptance',changedChecks:new Set()});
    const policies = [];
    ctx.verifyFrozenStage = async (_w, capability) => capability.includes('acceptance') ? acceptance : acceptance;
    ctx.authorCandidate = async (_w, policy) => { policies.push(policy); return skeleton; };
    // The second entry point repairs acceptance while verifying the skeleton.
    if (viaSkeleton) ctx.repairs.delete(acceptance);
    if (viaSkeleton) ctx.verifyFrozenStage = async (_w, capability) => {
      if (capability.includes('skeleton')) ctx.repairs.set(acceptance, {owner:'acceptance',changedChecks:new Set()});
      return acceptance;
    };
    await ctx.prepareAuthorStages({}, {acceptance:true,skeleton:true,subjects:[]});
    assert.equal(policies.length, 1);
    assert.equal(policies[0].directive, directive);
    assert.deepEqual([...policies[0].initialFeedback], []);
  }
});
