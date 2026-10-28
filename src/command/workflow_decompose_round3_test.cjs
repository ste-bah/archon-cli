// Round 3 regressions: run directly with node, before and after the fixes.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${scriptRoot}/${name}`, 'utf8')).join('\n');
function context(criteria = { A: 'a' }) {
  const logs = [];
  const ctx = { args: { acceptanceCriteria: criteria, gateMode: 'enforce', authorMaxParallelism: 1 },
    console: { log: line => logs.push(String(line)) },
    __archonHost: async (_, raw) => { throw new Error(JSON.parse(raw).reason); } };
  ctx.repairLogs = logs;
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  return ctx;
}
const answer = content => ({ status: 'accepted', stopReason: 'end_turn', content });
const clean = () => ({ publicationReceipt: { call_id: 'freeze' }, postcondition: { satisfied: true },
  gateEnvelope: { policy_findings: [] } });
function policy(ctx) {
  return { phase: 'acceptance', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']),
    shadowScopes: new Set(), author: (w, prompt, round, state) =>
      ctx.authorAcceptanceEntries(w, prompt, round, state) };
}
// Reworded findings do not count as progress. Each full stall window advances
// the authored ladder, and a later clean judge result completes the loop.
async function rewordedFindingsAdvanceTheLadder() {
  const ctx = context();
  let calls = 0;
  const prompts = [];
  const w = { checkpoint: async (_id, options) => { ctx.repairLogs.push(options.task); },  agent: async (_id, options) => {
      prompts.push(options.task);
      if (++calls > 500) throw new Error('spin: no pause after 500 calls');
      return answer('candidate');
    },
    hostCommand: async () => calls <= 13 ? ({ ...clean(), gateEnvelope: { policy_findings:
      [{ text: `defect reworded ${calls}`, remediation_scope: 'candidate_artifact' }] } }) : clean(),
    pause: async () => { throw new Error('authored finding requested a pause'); } };
  await ctx.authorCandidate(w, { phase: 'body', prompt: () => 'author',
    retryScopes: new Set(['candidate_artifact']), shadowScopes: new Set() });
  assert.equal(calls, 14);
  const transitions = ctx.repairLogs.filter(line => line.includes('repair rung'));
  assert.deepEqual(transitions.map(line => line.match(/r\d+/)[0]), ['r2', 'r3', 'r4', 'r2']);
  assert.deepEqual(transitions.map(line => Number(line.match(/attempts (\d+)/)[1])), [4, 7, 10, 13]);
  assert.match(prompts[4], /Re-author a complete replacement from fresh context/);
  assert.match(prompts[7], /Work through the findings in smaller groups/);
  assert.match(prompts[10], /deferred while independent subjects settle/);
  assert.match(prompts[13], /Re-author a complete replacement from fresh context/);
}
// A fresh VM has no volatile ladder state. Its host replays the durable
// requests and outcomes verbatim before dispatching the first new call.
async function midLadderReplayPreservesCallIdsPromptBytesAndRung() {
  async function execute(recorded = [], interrupt = false) {
    const ctx = context(), calls = [], replayed = [];
    const durable = JSON.parse(JSON.stringify(recorded));
    let cursor = 0, author = 0;
    const dispatch = async (method, id, options, result) => {
      const request = JSON.parse(JSON.stringify({method, id, options}));
      if (cursor < durable.length) {
        const record = durable[cursor++];
        assert.deepEqual(request, record.request, 'replay preserves call ids and prompt bytes');
        if (method === 'agent') assert.deepEqual(Buffer.from(options.task),
          Buffer.from(record.request.options.task), 'the UTF-8 prompt bytes replay exactly');
        replayed.push(record);
        calls.push(record);
        return JSON.parse(JSON.stringify(record.result));
      }
      if (interrupt && method === 'agent' && author === 9) {
        throw new Error('workflow cancelled by run control');
      }
      const record = {request, result};
      calls.push(record);
      return result;
    };
    const w = { checkpoint: async (id, options) => { ctx.repairLogs.push(options.task); return dispatch('checkpoint', id, options, {}); },
      agent: async (id, options) => {
        author++;
        return dispatch('agent', id, options, answer(`candidate-${author}`));
      },
      hostCommand: async (id, options) => dispatch('hostCommand', id, options,
        author <= 13 ? {...clean(), gateEnvelope:{policy_findings:[{
          text:`defect reworded ${author}`, subject:'TASK-X-001', remediation_scope:'candidate_artifact'
        }]}} : clean()),
      pause: async () => { throw new Error('authored finding requested a pause'); }
    };
    let error;
    try {
      await ctx.authorCandidate(w, {phase:'body', prompt:() => 'subject contract',
        capability:'land-task-body', retryScopes:new Set(['candidate_artifact'])});
    } catch (caught) { error = caught; }
    return {calls, replayed, logs:ctx.repairLogs, error};
  }
  const interrupted = await execute([], true);
  assert.match(interrupted.error?.message || '', /workflow cancelled by run control/);
  assert.equal(interrupted.calls.filter(call => call.request.method === 'agent').length, 8, 'eight author/gate pairs persist mid-r3');
  const resumed = await execute(interrupted.calls);
  const uninterrupted = await execute();
  assert.equal(resumed.error, undefined);
  assert.equal(uninterrupted.error, undefined);
  assert.deepEqual(resumed.replayed, interrupted.calls, 'the host returns recorded outcomes verbatim');
  assert.deepEqual(resumed.calls, uninterrupted.calls, 'every id, prompt byte and outcome matches a continuous run');
  assert.deepEqual(resumed.logs, uninterrupted.logs, 'the rung and cycle reconstruct by replay');
  const firstNew = resumed.calls[interrupted.calls.length].request;
  assert.equal(firstNew.id, 'body-author-9');
  assert.match(firstNew.options.task, /Work through the findings in smaller groups/);
  assert.deepEqual(resumed.logs.map(line => line.match(/r\d+/)[0]), ['r2', 'r3', 'r4', 'r2']);
}

async function partial(operational, replacing) {
  const ids = ['A', 'B', 'C', 'D'];
  const ctx = context(Object.fromEntries(ids.map(id => [id, id])));
  let gates = 0, evidence;
  const completed = [];
  const w = { checkpoint: async (_id, options) => { ctx.repairLogs.push(options.task); },  agent: async (callId) => {
    const [, id, ordinal] = /^acceptance-author-(\w+)-(\d+)$/.exec(callId);
    const round = Math.floor((Number(ordinal) - 1) / 3);
    if (replacing && round === 1) return answer(JSON.stringify({ id, version: 0 }));
    if (ids.indexOf(id) < round - (replacing ? 1 : 0)) {
      completed.push(id);
      return answer(JSON.stringify({ id, version: round }));
    }
    return operational ? { status: 'failed', summary: 'transport' } : answer('malformed');
  }, hostCommand: async () => replacing && ++gates === 1
    ? { ...clean(), gateEnvelope: { policy_findings: ids.map(id => ({
        text: `check '${id}': needs repair`, subject: id, remediation_scope: 'candidate_artifact' })) } }
  : clean(), pause: async (_id, value) => { evidence = value; throw new Error(`host retry window ${completed}`); } };
  if (replacing) {
    await ctx.authorCandidate(w, policy(ctx));
    assert.equal(gates, 2, 'a new malformed-reply refusal class advances once, then the repair reaches the gate');
  } else {
    await assert.rejects(ctx.authorCandidate(w, policy(ctx)), /host retry window/);
    assert.equal(gates, 0, 'a host retry window cannot dispatch a gate before all entries exist');
    assert.equal(evidence.reason, 'operational_no_progress');
    assert.doesNotMatch(JSON.stringify(evidence), /resume/i);
    assert.deepEqual(completed, ['A', 'B', 'C'], 'completed independent entries are retained before the host fault');
    assert.ok(!completed.includes('D'), 'the pending entry is never represented as completed');
  }
}
async function unchangedReplacement() {
  const ctx = context({ A: 'a', B: 'b' });
  // A kept entry always carries its host-owned criterion (Issue 357).
  const state = { entries: new Map([['A', { id: 'A', version: 0, criterion: 'a' }]]), retryIds: null };
  const w = { checkpoint: async (_id, options) => { ctx.repairLogs.push(options.task); },  agent: async id => id.includes('-A-')
    ? answer('{"version":0,"id":"A"}') : answer('malformed') };
  await ctx.authorAcceptanceEntries(w, 'author', 1, state);
  assert.equal(state.replaced, 0, 'reordered JSON is not a changed replacement');
  assert.equal(state.added, 0, 'an entry completed before is not a new one');
}
async function mixed() {
  const ctx = context();
  let calls = 0, evidence;
  const w = { checkpoint: async (_id, options) => { ctx.repairLogs.push(options.task); },  agent: async () => ++calls % 3 === 0 ? { status: 'failed', summary: 'transport' }
    : answer('malformed'), pause: async (_id, value) => { evidence = value; throw new Error('host retry window'); } };
  await assert.rejects(ctx.authorCandidate(w, policy(ctx)), /host retry window/);
  assert.equal(evidence.reason, 'operational_no_progress');
  assert.equal(calls, 9, `the mixed window records malformed replies and the host result before stopping: ${calls}`);
  assert.ok(ctx.repairLogs.some(line => line.includes('repair rung r2')), 'the malformed reply class advances the author ladder');
}
(async () => {
  let failed = 0;
  for (const [name, test] of [ ['reworded findings advance the ladder', rewordedFindingsAdvanceTheLadder],
    ['mid-ladder replay preserves call ids, prompt bytes and rung', midLadderReplayPreservesCallIdsPromptBytesAndRung],
    ['finding 2A replacements', () => partial(false, true)],
    ['operational retry window retains completed entries', () => partial(true, false)],
    ['finding 2 unchanged replacement', unchangedReplacement],
    ['mixed author refusals preserve the host retry window', mixed] ]) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
