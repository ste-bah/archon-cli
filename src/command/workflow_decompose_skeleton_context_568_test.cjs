const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');
const { authorContext, withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js", 'workflow_decompose_v1_progress.js',
  'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const digest = text => crypto.createHash('sha256').update(text).digest('hex');
const candidate = version => JSON.stringify({version, tasks:[{task_id:'TASK-X-001', file_name:'TASK-X-001.md',
  implements:['REQ-X-001'], deliverable_contracts:[{artifact_path:'out.txt'}], depends_on:[], blocks:[]}]});
const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  subjects:[{taskId:'TASK-X-001', fileName:'TASK-X-001.md'}], gateEnvelope:{policy_findings:[]}});
const finding = {text:'repair the named deliverable', remediation_scope:'skeleton'};
const refused = () => ({gateEnvelope:{policy_findings:[finding]}});
function context(phaseSeed) {
  const files = new Map();
  const ctx = {args:{prdPath:'/project/spec.md', taskRoot:'/project/tasks', gateMode:'enforce', phaseSeed},
    __archonAuthorContext:authorContext(files)};
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  return {ctx, files};
}
function assertPrevious(prompt, files, content) {
  const sha256 = digest(content);
  const path = `/run/author-context/${sha256}.json`;
  assert(prompt.includes(`The previous candidate is in ${path} (sha256 ${sha256}; host-written context for this call: read it by its exact path).`));
  assert.equal(files.get(path), content);
  assert(!prompt.includes(content), 'candidate bytes never inline');
  assert(prompt.includes('Whatever you restructure, keep every task, implements id, source_sections entry, deliverable and edge of the previous candidate that no current finding names. Compare the Landed subjects list at the end of this prompt with the previous candidate before landing changes and re-land every missing task with valid fields, preserving its work unless a current finding requires its removal. The host can drop invalid seeds. Only the subjects listed there have been retained. Re-land changed tasks.'));
  assert(!prompt.includes('Repair the previous candidate.'));
  assert(!prompt.includes('Change only what the current findings name.'));
  assert(Buffer.byteLength(prompt) <= 262144);
}
async function run(ctx, replies, gates, policy = ctx.skeletonPolicy([]), onPause) {
  const prompts = [], stdins = [];
  const outcome = await ctx.authorCandidate({
    agent:async (_id, options) => {
      prompts.push(options.task);
      assert(prompts.length <= replies.length, 'bounded author calls');
      return {status:'accepted', stopReason:'end_turn', retainedLandings:true, content:replies[prompts.length - 1]};
    },
    hostCommand:async (_id, options) => { stdins.push(options.stdin); return gates[stdins.length - 1]; },
    pause:async () => { if (onPause) onPause(); else throw new Error('unexpected pause'); }
  }, policy);
  return {prompts, stdins, outcome};
}

test('skeleton retries reference the latest judged candidate; the first prompt pins the incremental contract', async () => {
  const {ctx, files} = context();
  const replies = [candidate(1), candidate(2), candidate(3)];
  const out = await run(ctx, replies, [refused(), refused(), clean()]);
  assert(out.prompts[0].includes(ctx.deliverableContractRule()));
  // #599 adds the deliverable contract rule to the first prompt.
  // SHA-256 of the dispatched first prompt without #568 (main plus #569, the #572 obligation sentence, the #574 table-row constraint wording and the #575 source_sections shape field), with these exact args.
  // P5 replaces the whole-document prompt with one-task landing and exact receipt wording.
  // Pin the new condition explicitly; preserve the existing prompt golden.
  const landingRule = 'A call with open findings must make at least one byte-changing landing; a call with an empty task store must land at least one task. The gate then judges the whole set again.';
  assert(out.prompts[0].includes(landingRule));
  // S4a adds exactly the deliverable implements rule sentence and example line.
  // Assert both additions, then remove them to preserve the existing prompt pin.
  const implementsRule = " List in implements the ids from this task's implements that this deliverable carries.";
  const implementsExample = '"implements":["<id from this task\'s implements this artifact carries>"],';
  assert.equal(out.prompts[0].split(implementsRule).length, 2);
  assert.equal(out.prompts[0].split(implementsExample).length, 2);
  const baseline = out.prompts[0].replace(implementsRule, '').replace(implementsExample, '').replace(landingRule, 'Every author call must make at least one byte-changing landing; the gate then judges the whole set again.');
  assert.equal(digest(baseline), 'b43667c9511cc122a61162b4f2b8acc3345ee60c7a4c350c9394d558f8ae4368');
  assert(!out.prompts[0].includes('The previous candidate is in'));
  assert.equal(files.size, 2);
  assertPrevious(out.prompts[1], files, replies[0]);
  assertPrevious(out.prompts[2], files, replies[1]);
});

test('seeded refusal carries the durable candidate into its first repair prompt', async () => {
  const previous = candidate(0);
  const {ctx, files} = context({subjects:{skeleton:{kind:'artifact', candidate:previous, gate:{findings:[finding]}}}});
  const out = await run(ctx, [candidate(1)], [clean()]);
  assertPrevious(out.prompts[0], files, previous);
});

for (const frozen of [false, true]) test(`set-gate skeleton re-author ${frozen ? 'omits verified frozen report' : 'carries latest authored candidate'}`, async () => {
  const {ctx, files} = context();
  const previous = candidate(0);
  const outcome = clean();
  if (frozen) outcome.gateEnvelope.report = JSON.stringify({acceptance:{}, skeleton:{tasks:[]},
    subjects:outcome.subjects, bodies:[], tasks:[]});
  else ctx.recordSkeletonTasks(outcome, previous);
  const chain = {outcome, subjects:outcome.subjects};
  const prompts = [];
  ctx.authorBodies = async () => {};
  await ctx.reauthorSkeleton({
    agent:async (_id, options) => { prompts.push(options.task); return {status:'accepted', stopReason:'end_turn', retainedLandings:true, content:candidate(1)}; },
    hostCommand:async () => clean()
  }, chain, new Map(), [finding], [], 2);
  assert.equal(prompts.length, 1);
  if (frozen) {
    assert(!prompts[0].includes('The previous candidate is in'));
    assert.equal(files.size, 0);
    assert(prompts[0].includes('When the task root already holds a frozen skeleton, keep every task_id'));
  } else assertPrevious(prompts[0], files, previous);
});

test('missing content adds no prompt line or context file', () => {
  const {ctx, files} = context();
  const policy = ctx.skeletonPolicy([]);
  for (const missing of [undefined, null, '', {}]) assert.equal(policy.prompt(missing), policy.prompt());
  assert.equal(files.size, 0);
});

test('large previous skeleton stays in the context file', async () => {
  const {ctx, files} = context();
  const previous = JSON.stringify({tasks:[], padding:'x'.repeat(300000)});
  const out = await run(ctx, [candidate(1)], [clean()], ctx.skeletonPolicy([finding.text], 1, previous));
  assertPrevious(out.prompts[0], files, previous);
  assert(Buffer.byteLength(out.prompts[0]) < 16000);
});


test('resuming a stalled loop retains the last judged candidate', async () => {
  const {ctx, files} = context();
  const replies = [1, 2, 3, 4, 5].map(candidate);
  let pauses = 0;
  const out = await run(ctx, replies, [refused(), refused(), refused(), refused(), clean()],
    ctx.skeletonPolicy([]), () => { pauses += 1; });
  assert.equal(pauses, 1);
  assertPrevious(out.prompts[4], files, replies[3]);
});

test('previous candidate instructions accompany every repair rung', async () => {
  for (const rung of [1, 2, 3, 4]) {
    const {ctx, files} = context();
    const previous = candidate(0);
    const out = await run(ctx, [candidate(1)], [clean()], ctx.skeletonPolicy([finding.text], rung, previous));
    const prompt = out.prompts[0];
    assertPrevious(prompt, files, previous);
    assert(prompt.includes(`Repair operation r${rung},`));
    if (rung === 2) assert(prompt.includes('Re-author a complete replacement from fresh context. Keep the subject contract and repair every current finding.'));
    if (rung === 3) {
      assert(prompt.includes('Return a complete replacement. Work through the findings in smaller groups while preserving every finding from this and earlier attempts.'));
      assert(prompt.includes('Prioritize this group first:'));
    }
    if (rung === 4) assert(prompt.includes('This subject is deferred while independent subjects settle. Return a complete replacement when re-entered with the latest findings.'));
  }
});

test('carried seed reaches freeze without writing context or calling the author', async () => {
  const previous = candidate(0);
  const {ctx, files} = context({subjects:{skeleton:{kind:'artifact', candidate:previous}}});
  let writes = 0;
  ctx.__archonAuthorContext = () => { writes += 1; throw new Error('context write unavailable'); };
  const out = await run(ctx, [], [clean()]);
  assert.equal(writes, 0);
  assert.equal(files.size, 0);
  assert.deepEqual(out.prompts, []);
  assert.deepEqual(out.stdins, [previous]);
});

for (const scope of ['skeleton', 'body']) {
  for (const skeletonChanges of scope === 'skeleton' ? [false, true] : [false]) {
    test(`#587 ${scope} finding, skeleton ${skeletonChanges ? 'changes' : 'unchanged'}: named body receives omission and converges without pause`, async () => {
      const {ctx} = context();
      const previous = candidate(0);
      const outcome = clean();
      ctx.recordSkeletonTasks(outcome, previous);
      const chain = {outcome, subjects:outcome.subjects};
      const bodies = new Map([['TASK-X-001.md', {content:'a temporary root leaves the shared store empty'}]]);
      const text = 'task TASK-X-001: require the shared store entry; a temporary root leaves the shared store empty';
      const defect = {text, subject:'AC-X-001', source_path:'/project/tasks/TASK-X-001.md', remediation_scope:scope};
      let skeletonAuthors = 0, bodyAuthors = 0, lintCalls = 0, repaired = false;
      const w = {
        agent:async (id, options) => {
          if (id.startsWith('skeleton-')) {
            skeletonAuthors++;
            assert.equal(scope, 'skeleton');
            assert(options.task.includes(text), 'skeleton receives the finding');
            // C6: a zero-landing unchanged attempt is refused; the retry lands a changed task.
            const next = JSON.parse(previous);
            if (skeletonChanges || skeletonAuthors > 1) next.tasks[0].source_sections = ['1'];
            return {status:'accepted', stopReason:'end_turn', retainedLandings:skeletonChanges || skeletonAuthors > 1, content:JSON.stringify(next)};
          }
          bodyAuthors++;
          assert(options.task.includes(text), 'named body receives the finding even with identical frozen fields');
          repaired = true;
          return {status:'accepted', stopReason:'end_turn', content:'# Require the shared store entry'};
        },
        hostCommand:async capability => {
          const result = clean();
          if (capability === 'task-set-lint') {
            assert(++lintCalls <= 3, 'bound a regression that fails to reach the body');
            // Scripted critic stays false until the body changes, regardless of skeleton edits.
            result.gateEnvelope.policy_findings = repaired ? [] : [defect];
          }
          return result;
        },
        checkpoint:async () => ({}),
        pause:async () => { throw new Error('body omission must converge without pause'); }
      };
      await ctx.runSetGateLoop(w, chain, bodies);
      assert.equal(lintCalls, 2);
      assert.equal(skeletonAuthors, scope === 'skeleton' ? (skeletonChanges ? 1 : 2) : 0);
      assert.equal(bodyAuthors, 1);
      assert.equal(ctx.skeletonCandidateContent(chain.outcome), scope === 'body' ? previous
        : JSON.stringify({...JSON.parse(previous), tasks:[{...JSON.parse(previous).tasks[0], source_sections:['1']}]}));
    });
  }
}
