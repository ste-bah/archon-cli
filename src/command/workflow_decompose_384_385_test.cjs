// Issue 384 prompt rules and Issue 385 acceptance refusal/retry behavior.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const context = () => {
  const ctx = {args:{prdPath:'PRD.md', taskRoot:'tasks', repositoryRoot:'repo', projectRoot:'project',
    acceptanceCriteria:{A:'criterion'}, authorMaxParallelism:1, gateMode:'enforce'}};
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  return ctx;
};

function promptRules() {
  const ctx = context();
  const body = ctx.bodyPolicy({taskId:'TASK-X-001',fileName:'TASK-X-001.md'}, []).prompt();
  const acceptance = ctx.acceptanceAuthorPrompt();
  for (const rule of [
    'unquoted canonical TASK-<AREA>-<NNN>; its value must equal the TASK-<AREA>-<NNN> id in this filename, without the extension.',
    'Copy the frozen implements array exactly, preserving every value and its order.',
    'Every test file run by Focused Tests must be declared by an owning task',
    'Files Forbidden to Change entries must be literal paths, directories, basenames or globs',
    'Every MCP tool called by a Focused Tests command must appear in required_tools',
    'For each claimed PRD obligation, decide whether it is necessarily true',
  ]) assert.ok(body.includes(rule), `body prompt lacks rule: ${rule}`);
  assert.ok(acceptance.includes('fail on the pre-implementation baseline'), 'acceptance prompt lacks baseline proof rule');
  assert.ok(acceptance.includes('moves aside every data file it names by a path relative to its working directory'), 'acceptance prompt lacks already-true proof rule');
}

async function replyFormatIsTheLastTaskSection() {
  const ctx = context();
  const tasks = [];
  const state = {entries:new Map(), retryIds:new Set(['A'])};
  const w = {agent:async (_id, options) => {
    tasks.push(options.task);
    return {status:'accepted',stopReason:'end_turn',content:'{"id":'};
  }};
  await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 1, state);
  state.retryIds = new Set(['A']);
  await ctx.authorAcceptanceEntries(w, ctx.acceptanceAuthorPrompt(), 2, state);
  assert.equal(tasks.length, 2);
  assert.ok(tasks[0].includes('"command_block":true'), 'the acceptance entry shape uses a check block');
  assert.ok(!tasks[0].includes('"command":"'), 'the acceptance entry shape has no inline command example');
  const format = ctx.acceptanceReplyFormat();
  const formatLines = format.split('\n');
  assert.ok(formatLines.includes('```json'), 'the reply example opens a JSON fence on its own line');
  assert.ok(formatLines.includes('```check'), 'the reply example opens a check fence on its own line');
  assert.ok(formatLines.filter(line => line === '```').length >= 2, 'the reply example closes both fences on their own lines');
  assert.ok(format.includes('Every fence line is alone on its line: open with exactly ```json or ```check, close with exactly ```; never put text after a fence on the same line.'));
  for (const line of formatLines) {
    if (line.startsWith('```')) assert.ok(['```json','```check','```'].includes(line), `unexpected fence line: ${line}`);
  }
  for (const task of tasks) {
    assert.ok(task.includes('Reply format: see the end of this task.'));
    assert.ok(task.endsWith(format), 'the complete reply-format block is the final task section');
    assert.ok(!task.endsWith(`\n\`\`\`\n${format}`), 'the format is not wrapped in an outer fence');
  }
  assert.match(tasks[1], /The host refused this entry's last answered reply[\s\S]*JSON parse error/);
}

async function gateOutageReusesCandidate() {
  const ctx = context();
  let authors = 0, gates = 0;
  const candidate = '# authored task';
  const output = await ctx.authorCandidate({
    agent: async () => { authors++; return {status:'accepted',stopReason:'end_turn',content:candidate}; },
    hostCommand: async (_capability, request) => {
      gates++;
      assert.equal(request.stdin, candidate, 'the exact authored body reaches each gate attempt');
      return gates === 1 ? {gateEnvelope:{operational_error:{text:'temporary outage'}}} : {
        publicationReceipt:{call_id:'freeze'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}};
    },
    pause: async () => { throw new Error('unexpected pause'); },
  }, {phase:'body-TASK-X-001',prompt:()=>'',capability:'land-task-body',retryScopes:new Set(['body'])});
  assert.equal(authors, 1, 'an operational gate error must not call the author again');
  assert.equal(gates, 2);
  assert.equal(output.publicationReceipt.call_id, 'freeze');
}

async function wrongIdRefusalAdvancesTheLadderByItsFailureClass() {
  const ctx = context(); let calls = 0; const rungs = [], history = [];
  const original = ctx.recordRepairs;
  ctx.recordRepairs = (progress, ...params) => {
    const result = original(progress, ...params);
    history.push(JSON.parse(JSON.stringify(progress.history.at(-1)))); return result;
  };
  await ctx.authorCandidate({
    checkpoint: async (_id, options) => { rungs.push({text:options.task,calls,history:history.slice()}); },
    agent: async () => ({status:'accepted',stopReason:'end_turn',content: ++calls <= 13 ? '{"id":"B"}' : '{"id":"A"}'}),
    hostCommand: async () => ({publicationReceipt:{call_id:'published'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}}),
    pause: async () => {throw new Error('unexpected authored pause');}
  }, {phase:'acceptance',prompt:()=>'',author:ctx.authorAcceptanceEntries,capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])});
  assert.equal(calls,14);
  assert.equal(rungs[0].calls,4);
  assert.deepEqual(rungs[0].history.map(step=>[step.findings,step.progress]), [[1,true],[1,false],[1,false],[1,false]]);
  assert(rungs[0].history.every(step=>step.entries[0].malformed_class === 'wrong-id'));
}

async function wrongStopReasonIsSpecific() {
  const ctx = context();
  const out = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'max_tokens',content:'{"id":"A"}'
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  assert.match(out.summary, /cut off an incomplete reply of 10 characters across 1 lines/);
}

async function maxTokenFeedbackMeasuresReplyShape() {
  const ctx = context();
  const csv = ['week,open,high,low,close,volume', ...Array.from({length: 8}, (_, i) =>
    `2026-01-${String(i + 1).padStart(2, '0')},1,2,0,1,100`)].join('\n');
  const out = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'max_tokens',content:csv
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  assert.match(out.summary, /(?:\d+) characters/);
  assert.match(out.summary, /9 lines/);
  assert.match(out.summary, /8 of 9 lines/);
  assert.match(out.summary, /generate.*inside the check with a loop or small program/i);
  assert.match(out.summary, /keep only what the criterion needs/i);
  const jsonLines = Array.from({length: 4}, (_, i) => JSON.stringify({week:`2026-W${i + 1}`, value:i})).join('\n');
  const jsonOut = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'max_tokens',content:jsonLines
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  assert.match(jsonOut.summary, /4 of 4 lines share the same literal one-object-per-line JSON shape/);
}

async function maxTokenFeedbackDoesNotMislabelProseOrEmpty() {
  const ctx = context();
  const invoke = content => ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'max_tokens',content
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  const proseText = 'éé\nThese are ordinary words.\nStill prose.';
  const prose = await invoke(proseText);
  assert.ok(prose.summary.includes(`${Array.from(proseText).length} characters`));
  assert.match(prose.summary, /3 lines/);
  assert.doesNotMatch(prose.summary, /repeated|CSV|generate.*loop/i);
  const empty = await invoke('');
  assert.match(empty.summary, /0 characters/);
  assert.match(empty.summary, /0 lines/);
  assert.doesNotMatch(empty.summary, /repeated|CSV|generate.*loop/i);
}

async function replyFormatGuidesGeneratedFixtures() {
  const format = context().acceptanceReplyFormat();
  assert.ok(format.includes('Generate large or repetitive fixture data with code rather than writing it literally.'));
}

async function skeletonAndBodyReplyFormatsGuideGeneratedFixtures() {
  const ctx = context(), missing = [];
  const sentence = 'Generate large or repetitive fixture data with code rather than writing it literally.';
  for (const policy of [ctx.skeletonPolicy([]), ctx.bodyPolicy({ taskId: 'TASK-X-001', fileName: 'TASK-X-001.md' }, [])]) {
    const prompts = [];
    await ctx.authorCandidate({
      agent: async (_id, options) => {
        prompts.push(options.task);
        return { status: 'accepted', stopReason: 'end_turn', content: '{}' };
      },
      hostCommand: async () => ({ publicationReceipt: { call_id: 'published' },
        postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] } }),
      pause: async () => { throw new Error('unexpected pause'); }
    }, policy);
    assert.equal(prompts.length, 1);
    if (!prompts[0].includes(sentence)) missing.push(policy.phase);
  }
  assert.deepEqual(missing, [], 'every dispatched author prompt must carry the fixture generation sentence');
}

async function candidateLoopUsesMeasuredMaxTokenFeedback() {
  const ctx = context();
  const tasks = [];
  let calls = 0;
  const outcome = await ctx.authorCandidate({
    agent:async (_id, options) => {
      tasks.push(options.task);
      calls++;
      return calls === 1
        ? {status:'accepted',stopReason:'max_tokens',content:'row,a,b\n1,2,3\n2,3,4'}
        : {status:'accepted',stopReason:'end_turn',content:'{}'};
    },
    hostCommand:async () => ({publicationReceipt:{call_id:'freeze'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}}),
    pause:async () => { throw new Error('unexpected pause'); },
  }, {phase:'skeleton',prompt:()=> 'author',retryScopes:new Set(['candidate_artifact'])});
  assert.equal(outcome.postcondition.satisfied, true);
  assert.equal(tasks.length, 2);
  assert.match(tasks[1], /3 lines/);
  assert.match(tasks[1], /2 of 3 lines/);
  assert.match(tasks[1], /generate.*inside the check with a loop or small program/i);
}

async function missingContentIsSpecific() {
  const ctx = context();
  const out = await ctx.authorAcceptanceEntries({agent:async () => ({
    status:'accepted',stopReason:'end_turn'
  })}, 'author', 1, {entries:new Map(),retryIds:new Set(['A'])});
  assert.match(out.summary, /stop reason was end_turn but content was missing or empty/);
}

(async () => {
  for (const [name, test] of [['prompt rules',promptRules],['reply format last on first and repair attempts',replyFormatIsTheLastTaskSection],['gate outage candidate reuse',gateOutageReusesCandidate],
    ['wrong id failure class advances the ladder',wrongIdRefusalAdvancesTheLadderByItsFailureClass],['specific stop reason',wrongStopReasonIsSpecific],
    ['measured max token feedback',maxTokenFeedbackMeasuresReplyShape],
    ['prose and empty max token feedback',maxTokenFeedbackDoesNotMislabelProseOrEmpty],
    ['generated fixture reply format',replyFormatGuidesGeneratedFixtures],
    ['skeleton and body reply formats guide generated fixtures',skeletonAndBodyReplyFormatsGuideGeneratedFixtures],
    ['candidate loop measured feedback',candidateLoopUsesMeasuredMaxTokenFeedback],
    ['specific missing content',missingContentIsSpecific]]) {
    await test(); console.log(`PASS ${name}`);
  }
})().catch(error => { console.error(`FAIL ${error.stack}`); process.exitCode = 1; });
