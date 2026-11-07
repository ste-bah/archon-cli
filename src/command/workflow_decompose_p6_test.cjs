const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {withAuthorContext} = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const files = ['v1', 'v1_body', 'reply_blocks', 'v1_acceptance', 'v1_set_gate', 'v1_frozen',
  'v1_progress', 'v1_failing_set', 'v1_seed', 'v1_context'];
function context(extra = {}) {
  const ctx = withAuthorContext({args: {projectRoot:'project', repositoryRoot:'repo',
    prdPath:'prd.md', prdDigest:'digest', taskRoot:'tasks', gateMode:'enforce',
    acceptanceCriteria:{'AC-X-001':'criterion'}, ...extra}});
  ctx.__archonValidateAcceptanceEntry = () => "[]";
  vm.createContext(ctx);
  vm.runInContext(files.map(n => fs.readFileSync(`${root}/workflow_decompose_${n}.js`, 'utf8')).join('\n'), ctx);
  return ctx;
}
const subject = {taskId:'TASK-X-010', fileName:'TASK-X-010.md'};
const finding = scope => ({text:`task TASK-X-010: ${scope} defect`, subject:subject.taskId,
  source_path:subject.fileName, remediation_scope:scope});
const committed = findings => ({publicationReceipt:{call_id:'ok'}, result:{data:{publicationReceipt:{call_id:'ok'}}}, postcondition:{satisfied:true},
  gateEnvelope:{policy_findings:findings}, subjects:[subject]});

test('P6_8_prompt_names_every_obligation_and_constraint', () => {
  assert(context().bodyPolicy(subject, []).prompt().includes('Name every frozen implements id (obligation or constraint) in a body sentence outside the yaml block that also names the file, symbol or command that keeps or proves it.'));
});
test('P6_9_body_prompt_has_no_weakest_task_answer_contract', () => {
  assert.doesNotMatch(context().bodyPolicy(subject, []).prompt(), /weakest|loophole/);
});
test('P6_10_host_stamps_all_frozen_fields_and_command_examples_match', () => {
  const ctx = context();
  const rule = vm.runInContext('BODY_FROZEN_RULE', ctx);
  assert.equal(rule, 'The host stamps implements, source_sections, depends_on, blocks and deliverable_contracts from the frozen skeleton; read them there');
  assert(ctx.bodyPolicy(subject, []).prompt().includes(rule));
  assert.doesNotMatch(ctx.bodyPolicy(subject, []).prompt(), /Copy the frozen|Preserve every frozen/);
  assert(ctx.acceptanceReplyFormat().includes('Command examples: {"kind":"command","cwd":"repo_root","command":"make test"} and {"kind":"command","cwd":"project_root","command":"./verify"}.'));
});
function loop(ctx, rounds) {
  let round = 0;
  const skeleton = [], bodies = [], events = [];
  ctx.runSetGate = async (_w, capability) => {
    const envelope = capability === 'task-set-lint' ? rounds[Math.min(round++, rounds.length - 1)] : {};
    return {capability, routed:ctx.routeFindings({gateEnvelope:envelope}, new Set(['body']), new Set(['skeleton']))};
  };
  ctx.reauthorSkeleton = async (_w, _chain, _bodies, findings, bodyFindings) => {
    skeleton.push(Array.from(findings)); bodies.push(Array.from(bodyFindings));
  };
  ctx.authorBodies = async (_w, work) => bodies.push(Array.from(work));
  ctx.acceptSetGate = gate => gate;
  ctx.repairEvent = async () => {};
  const w = {checkpoint:async (id, options) => events.push({id, ...options})};
  return {run:ctx.runSetGateLoop(w, {subjects:[subject], outcome:{}}, new Map()), skeleton, bodies, events};
}
test('P6_11_unusable_judge_reply_retries_only_the_gate', async () => {
  const ctx = context();
  const records = [];
  const original = ctx.recordAnswered;
  ctx.recordAnswered = (...args) => { records.push(args[2]); return original(...args); };
  const result = loop(ctx, [{operational_error:{kind:'authored_judge_reply', text:'bad judge'}}, {}]);
  await result.run;
  assert.deepEqual(records, ['authored_judge_reply']);
  assert.equal(result.skeleton.length, 0);
  assert.equal(result.bodies.length, 0);
});
test('P6_12_host_failure_keeps_the_operational_window', async () => {
  const ctx = context(); let records = 0;
  const original = ctx.recordOperational;
  ctx.recordOperational = (...args) => { records++; return original(...args); };
  const result = loop(ctx, [{operational_error:{kind:'gate_operational', text:'host down'}}, {}]);
  await result.run;
  assert.equal(records, 1);
  assert.equal(result.skeleton.length, 0);
});
test('P6_13_escalation_sends_only_skeleton_findings_and_body_count', async () => {
  const result = loop(context(), [
    {policy_findings:[finding('body'), finding('skeleton')]},
    {policy_findings:[finding('body'), finding('skeleton')]},
    {policy_findings:[finding('body'), finding('skeleton')]}, {}]);
  await result.run;
  assert.equal(result.skeleton.length, 3);
  const escalated = result.skeleton[2];
  assert.equal(escalated.length, 2);
  assert.equal(escalated[0].remediation_scope, 'skeleton');
  assert.equal(escalated[1].text, '1 body findings remain open; they are re-authored by the bodies');
  for (const work of result.bodies) assert.equal(work[0].remediation_scope, 'body');
});
test('P6_14_body_only_stall_never_reauthors_skeleton', async () => {
  const result = loop(context(), [
    {policy_findings:[finding('body')]}, {policy_findings:[finding('body')]},
    {policy_findings:[finding('body')]}, {}]);
  await result.run;
  assert.equal(result.skeleton.length, 0);
  assert.equal(result.bodies.length, 3);
});
test('P6_15_pending_body_shadow_reauthors_skeleton_then_bodies', async () => {
  const ctx = context(); let bodyCalls = 0, skeletonCalls = 0;
  const defect = {...finding('skeleton'), text:'uncited_section: skeleton must cite section',
    source_path:'tasks/task-skeleton.json'};
  const w = {
    agent:async id => ({status:'accepted', stopReason:'end_turn',
      content:id.startsWith('acceptance-') ? '{"id":"AC-X-001"}' : '# body'}),
    hostCommand:async capability => {
      if (capability === 'freeze-skeleton') skeletonCalls++;
      if (capability === 'land-task-body' && ++bodyCalls === 1) {
        return {gateEnvelope:{policy_findings:[defect]}}; // pending draft: no receipt
      }
      return committed([]);
    },
    finalReport:async () => ({}), checkpoint:async () => {}
  };
  await ctx.workflow(w);
  assert.equal(skeletonCalls, 2);
  assert.equal(bodyCalls, 2);
  // A pending re-author can already have an older published body.
  // C6: an unchanged zero-landing reply must be refused before a changed landing retries the body.
  const skeleton = JSON.stringify({tasks:[{task_id:subject.taskId, file_name:subject.fileName}]});
  const prior = committed([]);
  ctx.recordSkeletonTasks(prior, skeleton);
  const bodies = new Map([[subject.fileName, committed([])]]);
  bodyCalls = 0;
  let skeletonAttempts = 0;
  w.agent = async id => {
    if (!id.startsWith('skeleton-')) return {status:'accepted', stopReason:'end_turn', content:'# body'};
    const changed = ++skeletonAttempts > 1;
    return {status:'accepted', stopReason:'end_turn', retainedLandings:changed,
      content:changed ? JSON.stringify({tasks:[{...JSON.parse(skeleton).tasks[0], source_sections:['1']}]}) : skeleton};
  };
  await ctx.authorBodies(w, [[subject, []]], bodies, {outcome:prior, subjects:[subject]});
  assert.equal(bodyCalls, 2);
  assert.equal(skeletonAttempts, 2);
});
test('P6_16_unattributed_body_finding_is_logged_and_sent_as_skeleton_context', async () => {
  const unknown = {text:'unattributed content defect', remediation_scope:'body', source_path:'prd.md'};
  const result = loop(context(), [{policy_findings:[unknown]}, {}]);
  await result.run;
  assert.equal(result.skeleton[0][0], unknown);
  assert.match(result.events[0].id, /^finding_unattributed-/);
  assert.match(result.events[0].task, /unattributed content defect/);
});
test('P6_17_path_quote_and_subject_keep_their_precedence', () => {
  const ctx = context();
  for (const f of [{source_path:`tasks/${subject.fileName}`},
    {text:`task ${subject.taskId}: defect`}, {subject:subject.taskId}]) {
    assert.equal(ctx.findSubject(f, [subject]), subject);
    assert.equal(ctx.groupFindingsBySubject([f], [subject])[0][0], subject.fileName);
  }
});
test('P6_18_body_uses_the_acceptance_site_PATH_fact', () => {
  const line = 'Check site PATH: /site/bin';
  const ctx = context({acceptanceCheckSitePathLine:line});
  assert(ctx.bodyPolicy(subject, []).prompt().split('\n').includes(line));
  assert(ctx.acceptanceAuthorPrompt().split('\n').includes(line));
  assert.doesNotMatch(context().bodyPolicy(subject, []).prompt(), /undefined|null/);
});

function shadowHarness(sequence, initialRung = 1, extra = {}) {
  const ctx = context();
  const chain = {subjects:[subject], outcome:{}};
  const bodies = new Map(), rungs = [], pauses = [];
  let calls = 0;
  ctx.authorCandidate = async (_w, policy) => {
    if (policy.phase === 'skeleton') {
      rungs.push(policy.initialRung);
      assert(calls < 20, 'shadow repair must be bounded');
      return {subjects:[subject]};
    }
    const value = sequence[Math.min(calls++, sequence.length - 1)];
    if (value === null) return {accepted:true};
    const texts = Array.isArray(value) ? value : [value];
    return {skeletonFindings:texts.map(text => typeof text === 'string'
      ? {...finding('skeleton'), text} : text), bodyFindings:[]};
  };
  const w = {checkpoint:async () => {}, pause:async (_id, evidence) => {
    pauses.push({calls, evidence}); throw new Error('paused');
  }, ...extra};
  const resume = () => ctx.authorBodies(w, [[subject, [], initialRung]], bodies, chain);
  return {run:resume(), resume, chain, rungs, pauses};
}
test('P6_15_repeated_shadow_pauses_no_progress_after_three_attempts', async () => {
  const result = shadowHarness(['unknown_section: fix section']);
  await assert.rejects(result.run, /paused/);
  assert.equal(result.pauses[0].calls, 3);
  assert.equal(result.pauses[0].evidence.reason, 'no_progress');
  assert.deepEqual(Array.from(result.pauses[0].evidence.last_findings), ['unknown_section: fix section']);
  assert.deepEqual(result.rungs, [2, 3]);
});
for (const [variant, sequence] of [
  ['alternating', ['uncited_section: A', 'unknown_section: B']],
  ['number_only', ['unknown_section: 1', 'unknown_section: 2', 'unknown_section: 3']],
]) {
  test(`P6_15_${variant}_shadow_pauses_after_three_attempts`, async () => {
    const values = variant === 'alternating' ? [...sequence, ...sequence] : sequence;
    const result = shadowHarness(values, 3);
    await assert.rejects(result.run, /paused/);
    assert.equal(result.pauses[0].calls, 3);
    assert.equal(result.pauses[0].evidence.reason, 'no_progress');
    assert.deepEqual(Array.from(result.pauses[0].evidence.last_findings), [values[2]]);
    assert.deepEqual(result.rungs, [4, 2]);
  });
}
test('P6_15_shadow_count_drop_resets_the_window', async () => {
  const result = shadowHarness([['A', 'B'], ['B', 'C'], ['C'], ['D'], ['E'], ['F']]);
  await assert.rejects(result.run, /paused/);
  assert.equal(result.pauses[0].calls, 6);
  assert.equal(result.pauses[0].evidence.progress_history[3].progress, true);
});
test('P6_15_resume_mid_shadow_loop_keeps_the_window', async () => {
  const checkpoints = [];
  const result = shadowHarness(['A', 'B', 'A'], 1, {checkpoint:async id => {
    checkpoints.push(id);
    if (checkpoints.length === 2) throw new Error('interrupted');
  }});
  await assert.rejects(result.run, /interrupted/);
  assert.equal(result.chain.bodyShadowRepair.progress.calls, 2);
  await assert.rejects(result.resume(), /paused/);
  assert.equal(result.pauses[0].calls, 3);
  // A host resume replays from a fresh script and reconstructs the window.
  const replayed = [];
  const replay = shadowHarness(['A', 'B', 'A'], 1, {checkpoint:async id => replayed.push(id)});
  await assert.rejects(replay.run, /paused/);
  assert.equal(replay.pauses[0].calls, 3);
  assert.deepEqual(replayed, checkpoints);
});
test('P6_15_moved_or_later_stage_shadow_findings_do_not_reset_the_count_window', async () => {
  const sequence = ['shape', 'graph', 'freeze'].map((stage, i) => ({
    ...finding('skeleton'), text:`defect at line ${i}`, subject:`section-${i}`,
    deterministic_defect:{provenance:'host_validator', code:'unknown_section',
      subject:`section-${i}`, location:`line-${i}`, stage}
  }));
  const result = shadowHarness(sequence);
  await assert.rejects(result.run, /paused/);
  assert.equal(result.pauses[0].calls, 3);
  assert.deepEqual(Array.from(result.pauses[0].evidence.last_findings), ['defect at line 2']);
});
test('P6_15_shadow_repair_checkpoint_ids_are_unique_across_episodes', async () => {
  const ids = [];
  const result = shadowHarness(['A', null, 'A', null, 'A', null], 1,
    {checkpoint:async id => { assert(!ids.includes(id), id); ids.push(id); }});
  await result.run;
  await result.resume();
  await result.resume();
  assert.equal(ids.length, 3);
  assert(ids.every(id => id.startsWith('repair-body-skeleton-')));
});
test('P6_15_pending_subject_keeps_findings_and_rung_before_shadow_repair', async () => {
  const ctx = context();
  const pending = {taskId:'TASK-X-020', fileName:'TASK-X-020.md'};
  const chain = {subjects:[subject, pending], outcome:{}};
  const bodies = new Map([[pending.fileName, {old:true}]]), retried = [], pauses = [];
  let first = true;
  ctx.authorCandidate = async (_w, policy) => {
    if (policy.target.taskId === subject.taskId) return {skeletonFindings:[finding('skeleton')], bodyFindings:[]};
    if (first) { first = false; return {pendingAuthor:true, findings:['pending finding'], pendingRung:4}; }
    retried.push({feedback:Array.from(policy.initialFeedback), rung:policy.initialRung});
    return {repaired:true};
  };
  ctx.reauthorSkeleton = async (_w, _chain, actualBodies, _findings, _bodyFindings, rung) => {
    assert.equal(actualBodies.get(pending.fileName).repaired, true);
    assert.equal(rung, 3);
  };
  await ctx.authorBodies({checkpoint:async () => {}, pause:async (_id, evidence) => pauses.push(evidence)},
    [[subject, [], 2], [pending, ['prior finding'], 4]], bodies, chain);
  assert.equal(pauses[0].reason, 'authored_pending');
  assert.deepEqual(retried, [{feedback:['prior finding', 'pending finding'], rung:4}]);
});
test('P6_16_unattributed_checkpoint_ids_are_unique_across_reauthors', async () => {
  const ctx = context(), ids = [];
  const w = {checkpoint:async id => { assert(!ids.includes(id)); ids.push(id); }};
  for (let round = 0; round < 3; round++) {
    await ctx.routeUnattributed(w, [{text:'unattributed repeated defect'}], [], [subject], 'body');
  }
  assert.equal(ids.length, 3);
});
test('P6_15_real_land_task_body_shadow_loop_pauses_with_the_finding', async () => {
  const ctx = context(); let bodies = 0, skeletons = 0, evidence;
  const defect = {...finding('skeleton'), text:'unknown_section: repair the frozen citation',
    source_path:'tasks/task-skeleton.json'};
  const w = {
    agent:async id => ({status:'accepted', stopReason:'end_turn',
      content:id.startsWith('acceptance-') ? '{"id":"AC-X-001"}' : '# body'}),
    hostCommand:async capability => {
      if (capability === 'freeze-skeleton') skeletons++;
      if (capability === 'land-task-body') {
        bodies++; assert(bodies < 10);
        return {gateEnvelope:{policy_findings:[defect]}};
      }
      return committed([]);
    },
    checkpoint:async () => {}, pause:async (_id, value) => { evidence = value; throw new Error('paused'); }
  };
  await assert.rejects(ctx.workflow(w), /paused/);
  assert.equal(bodies, 3);
  assert.equal(skeletons, 3);
  assert.equal(evidence.reason, 'no_progress');
  assert.deepEqual(Array.from(evidence.last_findings), [defect.text]);
});

function pendingShadowHarness(counts) {
  const ctx = context();
  const sibling = {taskId:'TASK-X-020', fileName:'TASK-X-020.md'};
  const chain = {subjects:[subject, sibling], outcome:{}};
  let shadows = 0, pendingCalls = 0, skeletons = 0;
  const pauses = [];
  ctx.authorCandidate = async (_w, policy) => {
    if (policy.phase === 'skeleton') {
      assert(++skeletons < 10, 'pending siblings must not reset the shadow window');
      return {subjects:chain.subjects};
    }
    if (policy.target.taskId === sibling.taskId) {
      return ++pendingCalls % 2 ? {pendingAuthor:true, findings:['retry sibling'], pendingRung:2}
        : {accepted:true};
    }
    const count = counts[Math.min(shadows++, counts.length - 1)];
    return count ? {skeletonFindings:Array.from({length:count}, () => finding('skeleton')), bodyFindings:[]}
      : {accepted:true};
  };
  const w = {checkpoint:async () => {}, pause:async (_id, evidence) => {
    pauses.push({shadows, evidence});
    if (evidence.reason !== 'authored_pending') throw new Error('paused');
  }};
  return {run:ctx.authorBodies(w, chain.subjects.map(s => [s, []]), new Map(), chain), pauses, chain};
}
test('P6_15_pending_sibling_each_episode_preserves_shadow_stall_window', async () => {
  const result = pendingShadowHarness([1]);
  await assert.rejects(result.run, /paused/);
  const pause = result.pauses.at(-1);
  assert.equal(pause.shadows, 3);
  assert.equal(pause.evidence.reason, 'no_progress');
  assert.deepEqual(Array.from(pause.evidence.last_findings), [finding('skeleton').text]);
});
test('P6_15_pending_sibling_still_allows_shadow_count_progress', async () => {
  const result = pendingShadowHarness([3, 2, 1, 0]);
  await result.run;
  assert.equal(result.pauses.length, 4);
  assert(result.pauses.every(p => p.evidence.reason === 'authored_pending'));
  assert.equal(result.chain.bodyShadowRepair, null);
});
test('P6_15_nested_pending_skeleton_repair_rechecks_outer_shadow', async () => {
  const ctx = context();
  const sibling = {taskId:'TASK-X-020', fileName:'TASK-X-020.md'};
  const chain = {subjects:[subject, sibling], outcome:{}};
  let skeletons = 0, outerCalls = 0, siblingCalls = 0;
  ctx.authorCandidate = async (_w, policy) => {
    if (policy.phase === 'skeleton') { skeletons++; return {subjects:chain.subjects}; }
    if (policy.target.taskId === subject.taskId) {
      outerCalls++;
      return skeletons ? {accepted:true} : {skeletonFindings:[finding('skeleton')], bodyFindings:[]};
    }
    if (++siblingCalls === 1) return {pendingAuthor:true, findings:['retry'], pendingRung:2};
    return skeletons ? {accepted:true} : {skeletonFindings:[{...finding('skeleton'), subject:sibling.taskId}], bodyFindings:[]};
  };
  await ctx.authorBodies({checkpoint:async () => {}, pause:async () => {}},
    chain.subjects.map(s => [s, []]), new Map(), chain);
  assert.equal(skeletons, 1, 'outer findings from the previous skeleton must not trigger another repair');
  assert.equal(outerCalls, 3, 'outer shadow must be checked against the repaired skeleton');
  assert.equal(chain.bodyShadowRepair, null);
});

test('P6_599_contract_shadow_repairs_skeleton_once_and_only_reauthors_reached_bodies', async () => {
  const ctx = context();
  const consumer = {taskId:'TASK-X-020', fileName:'TASK-X-020.md'};
  const sibling = {taskId:'TASK-X-030', fileName:'TASK-X-030.md'};
  const subjects = [subject, consumer, sibling];
  const entries = subjects.map(s => ({task_id:s.taskId, file_name:s.fileName,
    depends_on:s === consumer ? [{task_id:subject.taskId}] : [],
    deliverable_contracts:s === subject ? [{kind:'report', artifact_path:'output/report.json'}] : []}));
  const old = {subjects};
  ctx.recordSkeletonTasks(old, JSON.stringify({tasks:entries}));
  const chain = {subjects, outcome:old};
  const keptSibling = {accepted:'sibling'};
  const bodies = new Map([[subject.fileName, {old:true}], [consumer.fileName, {old:true}],
    [sibling.fileName, keptSibling]]);
  const defect = {...finding('skeleton'), text:'add a falsifiable typed_verifier_command in task-skeleton.json',
    source_path:'tasks/task-skeleton.json', deterministic_defect:{code:'invalid_verifier',
      subject:subject.taskId, location:'deliverable_contracts/0/typed_verifier_command'}};
  let freezes = 0;
  const authored = [];
  ctx.authorCandidate = async (_w, policy) => {
    if (policy.phase === 'skeleton') {
      freezes++;
      assert(policy.initialFeedback.includes(defect.text));
      const next = {subjects};
      entries[0].deliverable_contracts[0].typed_verifier_command = 'grep -q required {artifact_path}';
      ctx.recordSkeletonTasks(next, JSON.stringify({tasks:entries}));
      return next;
    }
    authored.push(policy.target.taskId);
    assert(!policy.initialFeedback.includes(defect.text), 'body author cannot repair frozen fields');
    if (!freezes) return {skeletonFindings:[defect], bodyFindings:[]};
    return {accepted:true};
  };
  await ctx.authorBodies({checkpoint:async () => {}, pause:async () => { throw Error('unexpected pause'); }},
    [[subject, []]], bodies, chain);
  assert.equal(freezes, 1);
  assert.deepEqual(authored, [subject.taskId, subject.taskId, consumer.taskId]);
  assert.equal(bodies.get(sibling.fileName), keptSibling);
});
