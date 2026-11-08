// Issue 384 prompt rules and Issue 385 acceptance refusal/retry behavior.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js", 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js']
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
    'The host stamps implements, source_sections, depends_on, blocks and deliverable_contracts from the frozen skeleton; read them there',
    'Every test file run by Focused Tests must be declared by an owning task',
    'Files Forbidden to Change entries must be literal paths, directories, basenames or globs',
    'Every MCP tool called by a Focused Tests command must appear in required_tools',
    'Name every frozen implements id (obligation or constraint) in a body sentence outside the yaml block',
  ]) assert.ok(body.includes(rule), `body prompt lacks rule: ${rule}`);
  assert.ok(acceptance.includes('fail on the pre-implementation baseline'), 'acceptance prompt lacks baseline proof rule');
  assert.ok(acceptance.includes('moves aside every file (source, test or data) it reads by a path relative to its working directory'), 'acceptance prompt lacks already-true proof rule');
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
  const shapes = vm.runInContext('ENTRY_SHAPES', ctx);
  assert.ok(!shapes.includes('"command":"'), 'the acceptance entry shape uses check blocks');
  assert.ok(tasks[0].includes('Command examples: {"kind":"command","cwd":"repo_root","command":"make test"} and {"kind":"command","cwd":"project_root","command":"./verify"}.'), 'the P4 examples show both roots');
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

async function wrongIdRefusalHasSpecificMeasuredProgress() {
  const ctx = context();
  let calls = 0; const pauses = [];
  await assert.rejects(ctx.authorCandidate({
    agent: async () => { calls++; return {status:'accepted',stopReason:'end_turn',content:'{"id":"B"}'}; },
    hostCommand: async () => { throw new Error('wrong id must never reach freeze'); },
    pause: async (_phase, evidence) => { pauses.push(evidence); throw new Error('paused'); },
  }, {phase:'acceptance',prompt:()=>'',author:ctx.authorAcceptanceEntries,
    capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])}), /paused/);
  assert.equal(calls, 4, 'the first refusal class advances; three repeats exhaust the window');
  const steps = Array.from(pauses[0].progress_history, step => [step.findings,step.progress]);
  assert.deepEqual(steps, [[1,true],[1,false],[1,false],[1,false]]);
  assert.match(pauses[0].last_findings.join(' '), /expected id A, got id "?B"?/);
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

// Both resume paths carry D while repairing A/B/C. A malformed A must not
// defer B/C to another round, and only A is called again before freeze.
async function seededAndFrozenRepairsContinueAfterMalformedReply() {
  for (const mode of ['seeded', 'frozen']) {
    const ctx = context();
    ctx.args.acceptanceCriteria = {A:'a', B:'b', C:'c', D:'d'};
    const entries = ['A','B','C','D'].map(id => ({id, criterion:id.toLowerCase(), version:0, covers:[]}));
    const findings = ['A','B','C'].map(id => ({subject:id, text:`check '${id}' was refuted: repair`,
      source_path:'tasks/acceptance-contract.json', remediation_scope:'candidate_artifact'}));
    const policy = {phase:'acceptance', prompt:()=> 'author', author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])};
    if (mode === 'seeded') {
      ctx.args.phaseSeed = {transition_index:0, author_ordinals:{}, pause_ordinals:{}, subjects:{acceptance:{
        kind:'entries', candidate:JSON.stringify({entries, supplementary:[]}), replies:[], invalid:{}, carried:4,
        gates:[{call_id:'g', published:false, findings}]}}};
      ctx.applySeedOrdinals();
    }
    const calls = [], prompts = [];
    let freezes = 0;
    const w = {
      agent:async (_, options) => {
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        calls.push(id); prompts.push(options.task);
        if (calls.length > 4) throw new Error('unexpected author retry');
        return {status:'accepted', stopReason:'end_turn', content:calls.length === 1 ? '{"id":' : JSON.stringify({id, version:1, covers:[]})};
      },
      hostCommand:async (_, request) => {
        freezes++;
        const submitted = JSON.parse(request.stdin);
        assert.deepEqual(submitted.entries.map(e => [e.id, e.version]), [['A',1],['B',1],['C',1],['D',0]]);
        return {publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true}, gateEnvelope:{policy_findings:[]}};
      },
      pause:async () => { throw new Error('unexpected pause'); },
    };
    if (mode === 'seeded') await ctx.authorCandidate(w, policy);
    else await ctx.repairFrozenStage(w, {gateEnvelope:{report:JSON.stringify({acceptance:entries, supplementary:[]})}}, findings);
    assert.deepEqual(calls, ['A','B','C','A'], mode);
    const prior = task => task.split('Previously completed entries')[1].split('\n').filter(line => line.startsWith('- ')).map(line => line.split(' ')[1]);
    assert.deepEqual(prior(prompts[1]), ['A','C','D'], `${mode}: B sees pre-round siblings including previous best rewrites`);
    assert.deepEqual(prior(prompts[2]), ['A','B','D'], `${mode}: C sees pre-round siblings, excluding itself`);
    assert.equal(freezes, 1);
  }
}

(async () => {
  for (const [name, test] of [['prompt rules',promptRules],['reply format last on first and repair attempts',replyFormatIsTheLastTaskSection],['gate outage candidate reuse',gateOutageReusesCandidate],
    ['wrong id measured refusal',wrongIdRefusalHasSpecificMeasuredProgress],['specific stop reason',wrongStopReasonIsSpecific],
    ['measured max token feedback',maxTokenFeedbackMeasuresReplyShape],
    ['prose and empty max token feedback',maxTokenFeedbackDoesNotMislabelProseOrEmpty],
    ['generated fixture reply format',replyFormatGuidesGeneratedFixtures],
    ['skeleton and body reply formats guide generated fixtures',skeletonAndBodyReplyFormatsGuideGeneratedFixtures],
    ['candidate loop measured feedback',candidateLoopUsesMeasuredMaxTokenFeedback],
    ['specific missing content',missingContentIsSpecific],
    ['seeded and frozen repair continuation',seededAndFrozenRepairsContinueAfterMalformedReply]]) {
    await test(); console.log(`PASS ${name}`);
  }
})().catch(error => { console.error(`FAIL ${error.stack}`); process.exitCode = 1; });
