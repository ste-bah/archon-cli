const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const { context, criteria, entry } = require('./workflow_decompose_prompt_size_test.cjs');
const { authorContext } = require('./workflow_decompose_context_stub.cjs');

async function reducedAcceptance() {
  const all = criteria(120);
  all['AC-001'] = 'criterion of AC-001\ncontinued criterion';
  const ctx = context({ acceptanceCriteria: all });
  let supplied = 0;
  ctx.__archonNamedFiles = (_entry, _notes, room) => {
    supplied += 1;
    const files = [{ path: 'src/deciding.rs', content: 'fn deciding() {}' }];
    assert(room >= Buffer.byteLength(JSON.stringify(files)));
    return JSON.stringify(files);
  };
  let task;
  await ctx.authorOne({ agent: async (_id, options) => {
    task = options.task;
    return { status: 'failed', summary: 'captured' };
  } }, 'phase instructions\n' + 'r'.repeat(252000), 1, 'AC-001', all['AC-001'],
  [entry('AC-002', 2)], all, { entries: new Map(), order: Object.keys(all) });
  assert.equal(supplied, 1);
  assert(Buffer.byteLength(task) <= 262144);
  assert(task.includes('All criterion IDs and text (for consistency): [moved to context file]'));
  assert(task.includes('Answer once you have read the supplied named_files content'));
  assert(task.includes('"path":"src/deciding.rs"'));
  assert(task.endsWith(ctx.acceptanceReplyFormat()), 'reply format stays last');
  return task;
}

async function positionalFixture(polluted = false) {
  const ctx = context({});
  const optional = 'Sibling marker:\n' + 's'.repeat(9000);
  const obsolete = 'old optional text '.repeat(1000);
  if (polluted) {
    // Earlier calls must not make required bytes in this call optional.
    ctx.optionalAuthorContext('obsolete context', obsolete);
    await ctx.requireDispatchable({}, 'earlier', obsolete);
  }
  const required = optional + '\n' + obsolete + 'r'.repeat(230000);
  const blocks = [];
  const task = required + ctx.optionalAuthorContext('sibling context', optional, blocks, required.length);
  const reduced = await ctx.requireDispatchable({}, 'resume', task, blocks);
  assert(reduced.startsWith(required), 'identical bytes inside required text survive');
  assert(reduced.includes('Sibling marker: [moved to context file]'));
  return reduced;
}

async function determinismAcrossFreshProcess() {
  const fresh = spawnSync(process.execPath, [__filename, 'resume'], { encoding: 'utf8' });
  assert.equal(fresh.status, 0, fresh.stderr);
  assert.equal(await positionalFixture(true), fresh.stdout, 'resume and long-running process dispatch identical bytes');
}

async function movedReferenceCanBeTheOnlyOverflow() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const stub = 'History: [moved to context file]';
  const required = 'r'.repeat(262144 - Buffer.byteLength(stub));
  const blocks = [];
  const optional = 'History:\n' + 'h'.repeat(4000);
  const task = required + ctx.optionalAuthorContext('prior history', optional, blocks, required.length);
  const reduced = await ctx.requireDispatchable({}, 'reference-only', task, blocks);
  assert(reduced.startsWith(required));
  assert(Buffer.byteLength(reduced) <= 262144);
  assert([...files.values()].some(text => JSON.parse(text)[0].text === optional));
}

async function refusalFor(ctx, task, blocks) {
  let reason;
  ctx.__archonHost = async (_action, payload) => { reason = JSON.parse(payload).reason; throw new Error('terminal refusal'); };
  await assert.rejects(ctx.requireDispatchable({}, 'required', task, blocks), /terminal refusal/);
  return reason;
}

async function currentFindingsMoveWithBoundedSubset() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const findings = ['first short finding\n- list item within the same finding', 'finding quoting "The previous candidate is in " ' + '東京🚀'.repeat(40000)];
  const blocks = [];
  const base = ctx.requiredAuthorContext('phase prompt', 'skeleton phase', blocks);
  const task = ctx.authorPrompt(base, 2, findings, [], blocks);
  const reduced = await ctx.requireDispatchable({}, 'current', task, blocks);
  assert(reduced.startsWith(base));
  assert(Buffer.byteLength(reduced) <= 262144);
  assert(reduced.includes('2 findings in full context; first entries that fit:\n- first short finding'));
  assert(!reduced.includes(findings[1]));
  const path = reduced.match(/Moved optional blocks: .* Read (\S+) /)[1];
  const stored = JSON.parse(files.get(path));
  assert.equal(stored[0].name, 'current authoritative findings');
  assert(stored[0].text.includes(findings.join('\n- ')));
}

async function refusalCountsOnlyFixedScaffold() {
  const ctx = context({});
  const blocks = [];
  const base = ctx.requiredAuthorContext('phase prompt', 'phase instructions\n' + 'r'.repeat(263000), blocks);
  const previous = ctx.previousSkeletonContext('{"tasks":[]}', blocks, base.length);
  const reason = await refusalFor(ctx, base + previous, blocks);
  assert(reason.includes('fixed scaffold (phase prompt)'), reason);
  assert(reason.includes(`uses ${Buffer.byteLength(base)} UTF-8 bytes`), reason);
  assert(reason.includes('limit 262144 bytes. This limit is not configurable.'), reason);
  assert(!reason.includes('named-files instructions'), reason);
  assert(!reason.includes('previous candidate reference'), reason);
  for (const block of blocks) assert.equal((base + previous).slice(block.start, block.start + block.text.length), block.text);
  const repair = ctx.bodyRepairContext({ capability: 'land-task-body' }, 1, {
    body: 'previous body', findings: [{ text: 'defect' }], score: 1, history: []
  });
  const bodyBlocks = repair.blocks.map(block => ({ ...block, start: base.length + block.start }));
  ctx.requiredAuthorContext('phase prompt', base, bodyBlocks);
  const bodyReason = await refusalFor(ctx, base + repair.text, bodyBlocks);
  assert(bodyReason.includes('retained candidate repair instructions'), bodyReason);
  assert(!bodyReason.includes('inline gate findings JSON'), bodyReason);
  assert(!bodyReason.includes('previous candidate reference'), bodyReason);
  const fixedBytes = Buffer.byteLength(base) + bodyBlocks.filter(block => block.required && block.name !== 'phase prompt')
    .reduce((sum, block) => sum + Buffer.byteLength(block.text), 0);
  assert(bodyReason.includes(`uses ${fixedBytes} UTF-8 bytes`), bodyReason);
  for (const block of bodyBlocks) assert.equal((base + repair.text).slice(block.start, block.start + block.text.length), block.text, block.name);

}

async function oversizedCriterionAndRungFourDefects() {
  const files = new Map();
  const all = { 'AC-001': 'c'.repeat(270000) };
  const ctx = context({ acceptanceCriteria: all }, authorContext(files));
  let dispatched;
  await ctx.authorOne({ agent: async (_id, options) => { dispatched = options.task; return { status: 'failed' }; } },
    'phase prompt', 1, 'AC-001', all['AC-001'], [], all, { entries: new Map(), order: ['AC-001'] });
  assert(Buffer.byteLength(dispatched) <= 262144);
  assert(dispatched.includes('Author ONLY entry AC-001: '));
  assert(dispatched.endsWith(ctx.acceptanceReplyFormat()));
  assert([...files.values()].some(text => text.includes(all['AC-001'])));
  const findings = Array.from({ length: 300 }, (_, i) => ({ text: 'defect ' + i + ' ' + 'd'.repeat(1000) }));
  const repair = ctx.bodyRepairContext({ capability: 'land-task-body' }, 4, { body: 'b', findings, score: 300, history: [] });
  const base = 'body phase';
  const blocks = repair.blocks.map(block => ({ ...block, start: base.length + block.start }));
  ctx.requiredAuthorContext('phase prompt', base, blocks);
  const task = ctx.authorPrompt(base + repair.text, 5, [], repair.history, blocks);
  const reduced = await ctx.requireDispatchable({}, 'rung-four', task, blocks);
  assert(Buffer.byteLength(reduced) <= 262144);
  assert(reduced.includes('300 findings in full context; first entries that fit:\n- defect 0 '));
  const path = reduced.match(/Moved optional blocks: .* Read (\S+) /)[1];
  const stored = JSON.parse(files.get(path));
  assert.equal(stored[0].name, 'previously seen defects');
  for (const finding of findings) assert(stored[0].text.includes(finding.text));
}

async function nearLimitOmitsNamedFilesWithoutReduction() {
  const ctx = context({});
  ctx.__archonNamedFiles = () => assert.fail('named files do not fit');
  const tail = '\nreply format';
  for (const spare of [0, 1, 100, 209]) {
    const task = 'r'.repeat(262144 - Buffer.byteLength(tail) - spare);
    const reserve = Buffer.byteLength(require('node:vm').runInContext('NAMED_FILES_PREAMBLE + "[]"', ctx));
    assert.equal(reserve, 210);
    const reduced = await ctx.requireDispatchable({}, 'boundary', task, [], tail);
    assert.equal(reduced, task, `preserve main behavior with ${spare} bytes free`);
    assert.equal(ctx.namedFilesContext(reduced, {}, [], tail), task + tail);
  }
  const optional = 'History:\n' + 'h'.repeat(20000);
  const reduce = async base => {
    const blocks = [];
    const task = base + ctx.optionalAuthorContext('prior history', optional, blocks, base.length);
    return ctx.requireDispatchable({}, 'reduced-boundary', task, blocks, tail);
  };
  const probe = await reduce('r'.repeat(250000));
  const reductionOverhead = Buffer.byteLength(probe) - 250000;
  const reduced = await reduce('r'.repeat(262144 - Buffer.byteLength(tail) - reductionOverhead - 100));
  assert.equal(Buffer.byteLength(reduced + tail), 262144 - 100);
  const sent = [];
  const dispatch = { agent: async (_id, options) => sent.push(options.task) };
  await dispatch.agent('boundary', { task: ctx.namedFilesContext(reduced, {}, [], tail) });
  assert.equal(sent[0], reduced + tail, 'reduced prompt dispatches without named-file instructions');
  ctx.__archonNamedFiles = () => '[]';
  // The same boundary through authorOne must dispatch without a false refusal.
  const all = { 'AC-X': 'criterion' };
  const captured = [];
  const w = { agent: async (_id, options) => { captured.push(options.task); return { status: 'failed' }; } };
  await ctx.authorOne(w, '', 1, 'AC-X', 'criterion', [], all, { entries: new Map(), order: ['AC-X'] });
  const overhead = Buffer.byteLength(captured.pop());
  await ctx.authorOne(w, 'r'.repeat(262144 - overhead + 210 - 100), 1, 'AC-X', 'criterion', [], all,
    { entries: new Map(), order: ['AC-X'] });
  assert.equal(captured.length, 1);
  assert(Buffer.byteLength(captured[0]) <= 262144);
  assert(!captured[0].includes('supplied named_files'));
}

async function rawCandidateIsWrittenOnlyWhenMoved() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const body = 'raw candidate contents '.repeat(400);
  const repair = ctx.bodyRepairContext({ capability: 'land-task-body' }, 1, { body, findings: [{ text: 'defect' }], score: 1, history: [] });
  const base = 'r'.repeat(245000);
  const blocks = repair.blocks.map(block => ({ ...block, start: base.length + block.start }));
  const own = base + repair.text;
  const task = own + ctx.optionalAuthorContext('sibling context', 'Siblings:\n' + 's'.repeat(20000), blocks, own.length);
  const reduced = await ctx.requireDispatchable({}, 'lazy-raw', task, blocks);
  assert(reduced.includes(body));
  assert(![...files.keys()].some(path => path.endsWith('.txt')), 'ranking an unmoved raw candidate writes no file');
}

async function smallCandidateMovesAsRawText() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const body = 'line with "quotes" and \\ slash\n'.repeat(400);
  assert(Buffer.byteLength(body) <= 16384);
  const repair = ctx.bodyRepairContext({ capability: 'land-task-body' }, 1, { body, findings: [{ text: 'defect' }], score: 1, history: [] });
  const base = 'r'.repeat(258000);
  const blocks = repair.blocks.map(block => ({ ...block, start: base.length + block.start }));
  const reduced = await ctx.requireDispatchable({}, 'raw', base + repair.text, blocks);
  const path = reduced.match(/Read the retained candidate as raw text at (\S+) /)[1];
  assert.equal(files.get(path), body);
  assert(path.endsWith('.txt'));
  assert(![...files.values()].some(text => text.includes(JSON.stringify(body))), 'no JSON-escaped candidate');
}

async function supersededDraftIsOptional() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const state = { superseded: new Map([['AC-001', { check: { command: 'old'.repeat(90000) } }]]) };
  const blocks = [];
  const own = ctx.acceptanceOwnContext(state, 'AC-001', blocks, 0);
  assert(Buffer.byteLength(own) > 262144);
  const reduced = await ctx.requireDispatchable({}, 'superseded', own, blocks);
  assert(Buffer.byteLength(reduced) < 262144);
  assert(reduced.includes('superseded draft'));
  assert([...files.values()].some(text => JSON.parse(text).some(block => block.name === 'superseded draft' && block.text === own)));
}

async function optionalEntryHistoryKeepsCriterion() {
  const all = criteria(120);
  all['AC-001'] = 'crit one';
  const ctx = context({ acceptanceCriteria: all });
  const state = { entries: new Map(), order: Object.keys(all),
    earlierGateFindings: new Map([['AC-001', Array.from({ length: 20 }, (_, i) => 'old finding ' + i + ' ' + 'x'.repeat(500))]]),
    earlierPhaseFindings: ['phase old ' + 'y'.repeat(3000)] };
  let task;
  await ctx.authorOne({ agent: async (_id, options) => { task = options.task; return { status: 'failed' }; } },
    'phase\n' + 'r'.repeat(245000), 1, 'AC-001', 'crit one', [entry('AC-002', 2)], all, state);
  assert(Buffer.byteLength(task) <= 262144);
  assert(task.includes('Author ONLY entry AC-001: crit one\n'));
  assert(task.includes('Moved optional blocks:'));
  assert(!task.includes('old finding 0 '));
}

async function normalAcceptanceVariantsKeepExactBytes() {
  for (const variant of [0, 1, 2, 3]) {
    const all = criteria(30);
    const ctx = context({ acceptanceCriteria: all });
    const id = 'AC-002';
    const prior = [entry('AC-001', 0), entry('AC-003', 2)];
    const state = { entries: new Map([[id, entry(id, 1)]]), order: Object.keys(all) };
    let own = '';
    if (variant >= 1) {
      state.refusals = new Map([[id, ['bad shape']]]);
      state.gateFindings = new Map([[id, ['gate f']]]);
      state.judgeNotes = new Map([[id, 'judge note']]);
      own += "\nThe host refused this entry's last answered reply. Repair exactly these findings:\n- bad shape"
        + "\nRepair this entry's gate findings open in the latest gate result:\n- gate f"
        + "\nThe freeze judge could not use this entry's last check:\n- judge note";
    }
    if (variant >= 2) {
      state.earlierGateFindings = new Map([[id, ['old a', 'old b']]]);
      state.phaseFindings = ['phase f'];
      state.earlierPhaseFindings = ['old phase'];
      state.superseded = new Map([[id, { id, x: 1 }]]);
      own += ctx.earlierFindingsText('Earlier gate results raised these findings for this entry; the latest gate result no longer reports them. Do not regress them:', ['old a', 'old b'])
        + '\nPhase-level findings of the latest gate result name no single entry; change this entry only where it causes one:\n- phase f'
        + ctx.earlierFindingsText('Earlier gate results raised these phase-level findings; the latest gate result no longer reports them. Do not regress them:', ['old phase'])
        + '\nA newer published version of this entry replaced your previous draft before it was published. Your previous draft was:\n' + JSON.stringify({ id, x: 1 });
    }
    own += ctx.currentCoversText(state.entries.get(id));
    const prompt = variant >= 3 ? ctx.authorPrompt('author', 3, [], []) : 'author';
    const expected = ctx.namedFilesContext(`${prompt}\nAuthor ONLY entry ${id}: ${all[id]}${own}\n${ctx.catalogueText(all, id, state.order)}\n${ctx.priorText(prior, id, state.order)}`,
      state.entries.get(id), [all[id], own, ...ctx.currentAuthorFindings(prompt)], `\n${ctx.acceptanceReplyFormat()}`);
    let actual;
    await ctx.authorOne({ agent: async (_id, options) => { actual = options.task; return { status: 'failed' }; } }, prompt, 1, id, all[id], prior, all, state);
    assert.equal(actual, expected, `normal acceptance variant ${variant}`);
  }
  const ctx = context({});
  const history = Array.from({ length: 12 }, (_, i) => ({ attempt: i + 1, findings: ['f' + i + ' ' + 'q'.repeat(300 * i)] }));
  const blocks = [];
  const historical = ctx.authorPrompt('skeleton base', 13, ['cur1', 'cur2'], history);
  assert.equal(ctx.authorPrompt('skeleton base', 13, ['cur1', 'cur2'], history, blocks), historical);
  assert.equal(await ctx.requireDispatchable({}, 'normal history', historical, blocks), historical);
  const body = 'line\n'.repeat(500);
  const findings = [{ text: 'defect a' }];
  const repair = ctx.bodyRepairContext({ capability: 'land-task-body' }, 1, { body, findings, score: 1, history: [] });
  assert.equal(repair.text, '\nCorrect the retained candidate below. Change only what its findings require and keep every other line exactly. Return the complete corrected file, never a patch.\n'
    + `Retained candidate (raw text):\n${body}\nEnd retained candidate.\nExact gate findings (JSON):\n${JSON.stringify({ findings, blocking_findings: 1 })}`);
}

const tests = [reducedAcceptance, determinismAcrossFreshProcess, movedReferenceCanBeTheOnlyOverflow, currentFindingsMoveWithBoundedSubset, refusalCountsOnlyFixedScaffold, nearLimitOmitsNamedFilesWithoutReduction, rawCandidateIsWrittenOnlyWhenMoved,
  oversizedCriterionAndRungFourDefects, smallCandidateMovesAsRawText, supersededDraftIsOptional, optionalEntryHistoryKeepsCriterion, normalAcceptanceVariantsKeepExactBytes];
module.exports = { tests };
if (require.main === module) {
  const mode = process.argv[2];
  (async () => {
    if (mode === 'seed' || mode === 'resume') process.stdout.write(await (mode === 'seed' ? reducedAcceptance() : positionalFixture()));
    else for (const test of tests) await test();
  })().catch(error => { console.error(error); process.exitCode = 1; });
}
