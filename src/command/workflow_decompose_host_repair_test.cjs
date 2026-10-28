const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const clean = () => ({ publicationReceipt: { call_id: 'published' }, postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] } });
const finding = (scope, source_path, text = 'repair deciding assertion') => ({
  subject: 'AC-X-001', source_path, text, remediation_scope: scope
});
function setup(extra = {}) {
  const ctx = withAuthorContext({ console: undefined, args: { gateMode: 'enforce',
    prdPath: '/input.md', taskRoot: '/subjects', acceptanceCriteria: { 'AC-X-001': 'criterion' } },
    __archonValidateAcceptanceEntry: () => '[]', ...extra });
  vm.createContext(ctx); vm.runInContext(source, ctx);
  const calls = [], events = [], pauses = [];
  ctx.__archonHost = async (_method, payload) => { throw new Error(JSON.parse(payload).reason); };
  const w = {
    agent: async (id, options) => {
      calls.push({ id, task: options.task });
      return { status: 'accepted', stopReason: 'end_turn', content: JSON.stringify({ id: 'AC-X-001' }) };
    },
    checkpoint: async (id, options) => { events.push({ id, ...options }); return {}; },
    pause: async (id, evidence) => { pauses.push({ id, evidence }); return { resumed: true }; }
  };
  const policy = { phase: 'acceptance', capability: 'freeze-acceptance', author: ctx.authorAcceptanceEntries,
    prompt: () => 'author', retryScopes: new Set(['candidate_artifact']) };
  return { ctx, w, calls, events, pauses, policy };
}
async function ownedSourceDispatchesAuthorBeforeNextGate() {
  const s = setup(); let gates = 0;
  s.ctx.args.acceptanceCriteria['AC-X-002'] = 'independent criterion';
  s.w.agent = async (id, options) => {
    s.calls.push({id,task:options.task});
    const entry = /Author ONLY entry ([^:]+):/.exec(options.task)[1];
    return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:entry})};
  };
  s.w.hostCommand = async () => ++gates < 8 ? { gateEnvelope: { policy_findings: [
    finding('unknown', '/subjects/acceptance-contract.json') ] } } : clean();
  await s.ctx.authorCandidate(s.w, s.policy);
  assert.equal(s.calls.length, gates + 1, 'each owned finding dispatches only its owner before the next freeze');
  assert.equal(s.calls.filter(call=>call.id.includes('AC-X-002')).length, 1, 'an accepted sibling is never reauthored');
  assert.equal(s.pauses.length, 0);
}
async function inputSourceStopsWithExactRemedy() {
  const s = setup(); let gates = 0;
  s.w.hostCommand = async () => { if (++gates > 8) throw new Error('repeated unowned freeze');
    return { gateEnvelope: { policy_findings: [finding('prd_input', '/input.md', 'duplicate AC-X-001 in /input.md; repair the duplicate input identifier')] } }; };
  await assert.rejects(s.ctx.authorCandidate(s.w, s.policy), /duplicate AC-X-001 in \/input.md; repair the duplicate input identifier/);
  assert.equal(gates, 1);
}
async function mixedRoutingPreservesTheUnownedFaultRemedy() {
  const s = setup();
  s.w.hostCommand = async () => ({gateEnvelope:{policy_findings:[
    finding('unknown', '/input.md', 'input artifact /input.md is incompatible; repair input format'),
    finding('operational', undefined, 'component unavailable; retry read'),
    finding('unknown', '/subjects/acceptance-contract.json')
  ]}});
  await assert.rejects(s.ctx.authorCandidate(s.w, s.policy), /input artifact \/input.md is incompatible; repair input format/);
  assert.equal(s.calls.length, 1);
}
async function freshContextKeepsAttributedGateFindings() {
  const s = setup(); let gates = 0;
  s.w.hostCommand = async () => ++gates <= 13 ? { gateEnvelope: { policy_findings: [finding('candidate_artifact')] } } : clean();
  await s.ctx.authorCandidate(s.w, s.policy);
  const fresh = s.calls.filter(call => call.task.includes('fresh context'));
  assert(fresh.length > 0);
  for (const call of fresh) assert.match(call.task, /repair deciding assertion/);
  assert(s.events.some(event => event.task.includes('r2')));
}
async function malformedCallsNeverRepeatAcrossThreeCycles() {
  const s = setup();
  s.w.agent = async (id, options) => {
    s.calls.push({ id, task: options.task });
    return { status: 'accepted', stopReason: 'end_turn', content: s.calls.length <= 33 ? 'unusable' : '{"id":"AC-X-001"}' };
  };
  s.w.hostCommand = async () => clean();
  await s.ctx.authorCandidate(s.w, s.policy);
  assert.equal(new Set(s.calls.map(call => call.task)).size, s.calls.length, 'every replacement carries its replayed attempt and cycle');
  assert(s.events.filter(event => /r2/.test(event.task)).length >= 3);
}
async function mixedUnprovenChecksRepairAuthoredEntries() {
  const s = setup(); let gates = 0;
  s.w.hostCommand = async () => ++gates === 1 ? { gateEnvelope: { policy_findings: [
    finding('operational', undefined, 'component unavailable; retry the component'), finding('candidate_artifact') ] } } : clean();
  await s.ctx.authorCandidate(s.w, s.policy);
  assert.equal(s.calls.length, 2);
  assert.match(s.calls[1].task, /repair deciding assertion/);
  assert.equal(s.pauses.length, 0);
}
async function frozenOperationalWindowRetriesWithHostRemedy() {
  const s = setup(); let gates = 0;
  s.w.hostCommand = async () => ++gates <= 4 ? { gateEnvelope: { operational_error: {
    text: 'component at /subjects/acceptance.lock unavailable; retry the read' } } } : clean();
  await s.ctx.verifyFrozenStage(s.w, 'verify-frozen-acceptance');
  assert.equal(gates, 5);
  assert(s.pauses.length > 0);
  assert.match(JSON.stringify(s.pauses), /\/subjects\/acceptance.lock/);
  assert.match(s.pauses[0].evidence.recovery, /retries/);
  assert.doesNotMatch(JSON.stringify(s.pauses), /restore|resume/i);
}
async function contextWriteFailuresUseOperationalWindow() {
  for (const acceptance of [true, false]) {
    const s = setup({ __archonAuthorContext: () => { throw new Error('component storage: disk full; retry write'); } });
    s.policy.prompt = () => 'x'.repeat(270000);
    if (!acceptance) delete s.policy.author;
    s.w.pause = async (_id, evidence) => {
      s.pauses.push(evidence);
      const error = new Error('controlled cancellation'); error.code = 'workflow_control'; error.kind = 'cancel'; throw error;
    };
    await assert.rejects(s.ctx.authorCandidate(s.w, s.policy), /controlled cancellation/);
    assert.equal(s.calls.length, 0);
    assert.match(JSON.stringify(s.pauses), /disk full/);
    assert.doesNotMatch(JSON.stringify(s.pauses), /resume/i);
  }
}
async function setGateBodyRepairsCarryTheOuterRung() {
  const s = setup({console: {log: () => {}}});
  const subject = { taskId: 'TASK-X-001', fileName: 'TASK-X-001.md' };
  let gates = 0;
  s.w.hostCommand = async capability => {
    if (capability === 'freeze-skeleton') return {...clean(), subjects:[subject]};
    if (capability !== 'task-set-lint') return clean();
    return ++gates <= 30 ? {gateEnvelope:{policy_findings:[{
      text:'repair assertion', subject:subject.taskId, source_path:'/subjects/TASK-X-001.md', remediation_scope:'body'
    }]}} : clean();
  };
  await s.ctx.runSetGateLoop(s.w, {outcome:clean(), subjects:[subject]}, new Map());
  const bodies = s.calls.filter(call => call.id.startsWith('body-'));
  assert(bodies.some(call => /Repair operation r2/.test(call.task)));
  assert(bodies.some(call => /Repair operation r3/.test(call.task)));
  assert(bodies.some(call => /Repair operation r4/.test(call.task)));
  assert.equal(new Set(bodies.map(call => call.task)).size, bodies.length);
}
async function midLadderReplayKeepsIdsPromptsAndRung() {
  const first = setup(), tape = [];
  first.w.agent = async (id, options) => {
    const result = { status: 'accepted', stopReason: 'end_turn', content: '{}' };
    tape.push({ kind: 'agent', id, options: JSON.parse(JSON.stringify(options)), result });
    return result;
  };
  let gates = 0;
  first.w.hostCommand = async (id, options) => {
    const result = ++gates <= 22 ? { gateEnvelope: { policy_findings: [finding('candidate_artifact')] } } : clean();
    tape.push({ kind: 'host', id, options: JSON.parse(JSON.stringify(options)), result }); return result;
  };
  const checkpoint = first.w.checkpoint;
  first.w.checkpoint = async (id, options) => { const result = await checkpoint(id, options);
    tape.push({ kind: 'event', id, options: JSON.parse(JSON.stringify(options)), result }); return result; };
  await first.ctx.authorCandidate(first.w, { ...first.policy, author: undefined });
  const boundary = tape.findIndex(call => call.kind === 'event' && call.options.task.includes('r3')) + 3;
  assert(boundary > 3);
  const replay = setup(); let index = 0;
  const replayCall = kind => async (id, options) => {
    const expected = tape[index++];
    assert.deepEqual({ kind, id, options: JSON.parse(JSON.stringify(options)) },
      { kind: expected.kind, id: expected.id, options: expected.options }, `call ${index}, replay prefix ends at ${boundary}`);
    return expected.result;
  };
  replay.w.agent = replayCall('agent'); replay.w.hostCommand = replayCall('host'); replay.w.checkpoint = replayCall('event');
  await replay.ctx.authorCandidate(replay.w, { ...replay.policy, author: undefined });
  assert.equal(index, tape.length);
  assert(tape.slice(boundary).some(call => call.kind === 'event' && call.options.task.includes('r4')));
}
async function inputBasenameCollisionStopsBeforeBodyRepair() {
  const s = setup(), subject = {taskId:'TASK-X-001', fileName:'TASK-X-001.md'};
  let gates = 0;
  s.ctx.args.prdPath = '/input/TASK-X-001.md';
  s.w.hostCommand = async capability => {
    if (capability === 'freeze-skeleton') return {...clean(), subjects:[subject]};
    if (capability !== 'task-set-lint') return clean();
    if (++gates > 8) throw new Error('controlled cancellation');
    return {gateEnvelope:{policy_findings:[finding('prd_input', s.ctx.args.prdPath,
      'duplicate identifier in /input/TASK-X-001.md; repair the input identifier')]}};
  };
  await assert.rejects(s.ctx.runSetGateLoop(s.w, {outcome:clean(), subjects:[subject]}, new Map()),
    /duplicate identifier in \/input\/TASK-X-001.md; repair the input identifier/);
  assert.equal(gates, 1);
  assert.equal(s.calls.length, 0);
}
async function bodyContextRecoveryReplaysRecordedIdsAndPromptBytes() {
  const recorded = setup(), binding = recorded.ctx.__archonAuthorContext;
  let writes = 0;
  recorded.ctx.__archonAuthorContext = (...args) => {
    if (++writes === 1) throw new Error('component storage unavailable; retry write');
    return binding(...args);
  };
  const subject = {taskId:'TASK-X-001', fileName:'TASK-X-001.md'};
  const feedback = ['repair assertion ' + 'x'.repeat(270000)];
  recorded.w.hostCommand = async () => clean();
  await recorded.ctx.authorCandidate(recorded.w, recorded.ctx.bodyPolicy(subject, feedback));
  assert(writes > 1, 'storage recovered before the body dispatch');
  const replay = setup(); let index = 0;
  replay.w.agent = async (id, options) => {
    const expected = recorded.calls[index++];
    assert.equal(id, expected.id, 'healthy-storage replay must preserve the recorded body call id');
    assert.equal(options.task === expected.task, true, 'healthy-storage replay must preserve every recorded prompt byte');
    return {status:'accepted',stopReason:'end_turn',content:'{}'};
  };
  replay.w.hostCommand = async () => clean();
  await replay.ctx.authorCandidate(replay.w, replay.ctx.bodyPolicy(subject, feedback));
  assert.equal(index, recorded.calls.length);
  assert.equal(recorded.calls[0].id, 'body-TASK-X-001-author-1');
  assert.deepEqual(replay.events, recorded.events, 'no unrecorded write changes the replayed rung');
}
async function bodyContextRecoveryReplaysRungCheckpointIds() {
  const recorded = setup(), binding = recorded.ctx.__archonAuthorContext, tape = [];
  let writes = 0, gates = 0;
  recorded.ctx.__archonAuthorContext = (...args) => {
    if (++writes === 1) throw new Error('component storage unavailable; retry write');
    return binding(...args);
  };
  const subject = {taskId:'TASK-X-001', fileName:'TASK-X-001.md'};
  const feedback = ['repair assertion ' + 'x'.repeat(270000)];
  const capture = (kind, result) => async (id, options) => {
    const value = result();
    tape.push({kind,id,options:JSON.parse(JSON.stringify(options)),result:value});
    return value;
  };
  recorded.w.agent = capture('agent', () => ({status:'accepted',stopReason:'end_turn',content:'{}'}));
  recorded.w.hostCommand = capture('host', () => ++gates <= 8
    ? {gateEnvelope:{policy_findings:[finding('body', '/subjects/TASK-X-001.md')]}} : clean());
  recorded.w.checkpoint = capture('event', () => ({}));
  await recorded.ctx.authorCandidate(recorded.w, recorded.ctx.bodyPolicy(subject, feedback));
  assert(tape.some(call => call.kind === 'event'), 'recovery must reach an authored rung transition');
  const replay = setup(); let index = 0;
  const replayCall = kind => async (id, options) => {
    const expected = tape[index++];
    assert.deepEqual({kind,id,options:JSON.parse(JSON.stringify(options))},
      {kind:expected.kind,id:expected.id,options:expected.options}, 'every recorded call must replay verbatim after context recovery');
    return expected.result;
  };
  replay.w.agent = replayCall('agent'); replay.w.hostCommand = replayCall('host'); replay.w.checkpoint = replayCall('event');
  await replay.ctx.authorCandidate(replay.w, replay.ctx.bodyPolicy(subject, feedback));
  assert.equal(index, tape.length);
}
async function acceptanceContextRecoveryReplaysEveryEntryAndRung() {
  for (const mode of ["entry", "sibling", "outer"]) {
    const failAfterSibling = mode === "sibling";
    const recorded = setup(), tape = [];
    recorded.policy.prompt = recorded.ctx.acceptanceAuthorPrompt;
    if (mode === "outer") recorded.policy.initialFeedback = ["repair assertion " + "x".repeat(270000)];
    recorded.ctx.args.acceptanceCriteria['AC-X-001'] = 'criterion ' + 'x'.repeat(270000);
    if (failAfterSibling) recorded.ctx.args.acceptanceCriteria['AC-X-002'] = 'independent criterion';
    const binding = recorded.ctx.__archonAuthorContext;
    let failed = false, gates = 0;
    recorded.ctx.__archonAuthorContext = (...params) => {
      const siblingDispatched = tape.some(call => call.kind === 'agent');
      if (!failed && (!failAfterSibling || siblingDispatched)) {
        failed = true;
        throw new Error('component storage unavailable; retry write');
      }
      return binding(...params);
    };
    const capture = (kind, result) => async (id, options) => {
      const value = result(id, options);
      tape.push({kind, id, options:JSON.parse(JSON.stringify(options)), result:value});
      return value;
    };
    // Compact dispatches point to exact host-written prompts: resolve that
    // context to return the requested entry, just as an author reads it.
    recorded.w.agent = capture('agent', (_id, options) => {
      const task = options.task.includes('Author ONLY entry') ? options.task
        : [...binding.files.values()].find(text => text.includes('Author ONLY entry') &&
          options.task.includes(require('node:crypto').createHash('sha256').update(text).digest('hex')));
      return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:/Author ONLY entry ([^:]+):/.exec(task)[1]})};
    });
    recorded.w.hostCommand = capture('host', () => ++gates <= 8
      ? {gateEnvelope:{policy_findings:[finding('candidate_artifact')]}} : clean());
    recorded.w.checkpoint = capture('event', () => ({}));
    await recorded.ctx.authorCandidate(recorded.w, recorded.policy);
    assert(failed, 'the entry preparation must encounter the storage failure');
    assert(tape.some(call => call.kind === 'event'), 'recovery reaches a repair rung');
    const replay = setup(); let index = 0;
    replay.policy.prompt = replay.ctx.acceptanceAuthorPrompt;
    replay.policy.initialFeedback = recorded.policy.initialFeedback;
    replay.ctx.args.acceptanceCriteria = recorded.ctx.args.acceptanceCriteria;
    const replayCall = kind => async (id, options) => {
      const expected = tape[index++];
      assert.equal(id, expected.id, 'healthy-storage replay preserves every acceptance call id');
      assert.equal(JSON.stringify(options), JSON.stringify(expected.options),
        'healthy-storage replay preserves every acceptance prompt byte and rung');
      assert.equal(kind, expected.kind);
      return expected.result;
    };
    replay.w.agent = replayCall('agent'); replay.w.hostCommand = replayCall('host'); replay.w.checkpoint = replayCall('event');
    await replay.ctx.authorCandidate(replay.w, replay.policy);
    assert.equal(index, tape.length);
  }
}
(async () => {
  let failed = 0;
  for (const test of [ownedSourceDispatchesAuthorBeforeNextGate, inputSourceStopsWithExactRemedy,
    mixedRoutingPreservesTheUnownedFaultRemedy, freshContextKeepsAttributedGateFindings, malformedCallsNeverRepeatAcrossThreeCycles,
    mixedUnprovenChecksRepairAuthoredEntries, frozenOperationalWindowRetriesWithHostRemedy,
    contextWriteFailuresUseOperationalWindow, setGateBodyRepairsCarryTheOuterRung, midLadderReplayKeepsIdsPromptsAndRung,
    inputBasenameCollisionStopsBeforeBodyRepair, bodyContextRecoveryReplaysRecordedIdsAndPromptBytes,
    bodyContextRecoveryReplaysRungCheckpointIds, acceptanceContextRecoveryReplaysEveryEntryAndRung]) {
    try { await test(); console.log(`PASS ${test.name}`); }
    catch (error) { failed++; console.error(`FAIL ${test.name}: ${error.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
