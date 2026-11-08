const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js", 'workflow_decompose_v1_progress.js',
  'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const entry = (id = 'A', extra = {}) => ({id, criterion:'criterion',
  check:{kind:'command', cwd:'project_root', command:'true'}, gap_permitted:false,
  judgment:{verdict:'accepted', counterexample:'', reason:'', host_call_id:''}, ...extra});
function context(phaseSeed) {
  const ctx = {args:{acceptanceCriteria:{A:'criterion'}, prdRequirementTexts:{'REQ-X':'requirement'},
    authorMaxParallelism:1, phaseSeed}, __archonValidateAcceptanceEntry:() => '[]'};
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  ctx.requireDispatchable = async (_w, _label, task) => task;
  ctx.catalogueText = ctx.priorText = () => '';
  return ctx;
}
async function author(ctx, state, reply, round = 1) {
  let task, call;
  const result = await ctx.authorAcceptanceEntries({agent:async (id, options) => {
    call = id; task = options.task;
    return {status:'accepted', stopReason:'end_turn', content:JSON.stringify(reply)};
  }}, 'author', round, state);
  return {result, task, call};
}
const state = () => ({entries:new Map(), retryIds:null});

test('entry shapes name the requirement prefix and match Rust apart from the P4 cwd handoff', () => {
  const ctx = context();
  const prefix = vm.runInContext('REQUIREMENT_PREFIX', ctx);
  const rustCoverage = fs.readFileSync(`${root}/../../crates/archon-workflow/src/v2/acceptance_coverage.rs`, 'utf8');
  assert.equal(prefix, rustCoverage.match(/pub const REQUIREMENT_PREFIX: &str = "([^"]+)";/)[1]);
  const shapes = vm.runInContext('ENTRY_SHAPES', ctx);
  const expected = `<${prefix} requirement id from the PRD whose violation makes this check fail>`;
  for (const shape of JSON.parse(shapes)) assert.deepEqual(shape.covers, [expected]);
  const rust = fs.readFileSync(`${root}/workflow_acceptance_reauthor.rs`, 'utf8');
  const template = rust.match(/static ENTRY_SHAPES:[\s\S]*?r#"([\s\S]*?)"#/)[1];
  // P4 updates the Rust prompt in its branch; P6 removes the single-root example here.
  const rustShapes = JSON.parse(template.replaceAll('{REQUIREMENT_PREFIX}', prefix));
  const command = JSON.parse(shapes).find(entry => entry.check.kind === 'command');
  assert.equal(command.check.cwd, '<repo_root or project_root>');
  rustShapes.find(entry => entry.check.kind === 'command').check.cwd = command.check.cwd;
  assert.deepEqual(JSON.parse(shapes), rustShapes);
  assert.ok(ctx.acceptanceAuthorPrompt().includes(`covers lists every one of the PRD's ${prefix} requirement ids (requirement bullets or obligation-table rows that start with ${prefix})`));
});

test('reply format includes covers between gap_permitted and judgment', () => {
  const format = context().acceptanceReplyFormat();
  const example = JSON.parse(format.split('\n').find(line => line.startsWith('{')));
  assert.deepEqual(example.covers, ['<REQ-ID>']);
  assert.match(format, /"gap_permitted":false,"covers":\["<REQ-ID>"\],"judgment"/);
});

test('re-author prompt shows current covers; a new entry has no coverage line', async () => {
  const ctx = context(), kept = entry('A', {covers:['REQ-X', 'REQ-Y']});
  const rewrite = await author(ctx, {entries:new Map([['A', kept]]), retryIds:null}, kept);
  assert.ok(rewrite.task.split('\n').includes('This entry currently covers ["REQ-X","REQ-Y"]. Keep every one of them in covers unless the new check no longer fails when that requirement is violated; the host re-opens each one you drop that no other check covers as an owed supplementary check.'));
  const fresh = await author(ctx, state(), entry('A', {covers:[]}));
  assert.doesNotMatch(fresh.task, /This entry currently covers/);
});

test('omitted covers inherits a copy of the current covers', async () => {
  const ctx = context(), prior = entry('A', {covers:['REQ-X', 'REQ-Y']});
  const s = {entries:new Map([['A', prior]]), retryIds:null};
  assert.equal((await author(ctx, s, entry())).result.status, 'accepted');
  assert.deepEqual([...s.entries.get('A').covers], ['REQ-X', 'REQ-Y']);
  assert.notEqual(s.entries.get('A').covers, prior.covers);
});

test('explicit empty covers replaces prior covers', async () => {
  const ctx = context(), s = {entries:new Map([['A', entry('A', {covers:['REQ-X', 'REQ-Y']})]]), retryIds:null};
  const out = await author(ctx, s, entry('A', {covers:[]}));
  assert.equal(out.result.status, 'accepted');
  assert.deepEqual([...s.entries.get('A').covers], []);
});

test('first-time omitted covers covers nothing; present non-array is not inherited', async () => {
  const ctx = context(), s = state();
  const out = await author(ctx, s, entry());
  assert.equal(out.result.status, 'accepted');
  assert.deepEqual(JSON.parse(out.result.content).entries[0].covers || [], []);
  for (const covers of [null, 'REQ-X', {}]) {
    const c = context(), current = {entries:new Map([['A', entry('A', {covers:['REQ-X']})]]), retryIds:null};
    assert.equal((await author(c, current, entry('A', {covers}))).result.status, 'accepted');
    assert.deepEqual(JSON.parse(JSON.stringify(current.entries.get('A').covers)), covers);
  }
});

test('owed supplementary replies may omit covers and receive their requirement', async () => {
  const ctx = context();
  ctx.owedSupplementary().set('SUP-REQ-X', {requirement:'REQ-X', text:'requirement'});
  const out = await author(ctx, {entries:new Map([['A', entry('A', {covers:[]})],
    ['SUP-REQ-X', entry('SUP-REQ-X', {covers:['REQ-Y']})]]), retryIds:new Set(['SUP-REQ-X'])}, entry('SUP-REQ-X'), 2);
  assert.equal(out.result.status, 'accepted');
  assert.deepEqual(JSON.parse(out.result.content).supplementary[0].covers, ['REQ-X']);
});

test('seed replay uses Rust-resolved covers without inferring from carried entries', async () => {
  for (const extra of [{}, {covers:[]}]) {
    const prior = entry('A', {covers:['REQ-X', 'REQ-Y']}), reply = entry('A', extra);
    const seed = {kind:'entries', carried_entries:[prior], gates:[],
      replies:[{id:'A', text:JSON.stringify(entry('A', {covers:extra.covers || prior.covers})), criterion:'criterion'}]};
    const ctx = context({subjects:{acceptance:seed}}), s = state();
    ctx.seedAuthorLoop({phase:'acceptance', author:true, retryScopes:new Set()}, s);
    const live = {entries:new Map([['A', prior]]), retryIds:null};
    await author(context(), live, reply);
    assert.deepEqual([...s.entries.get('A').covers], extra.covers || ['REQ-X', 'REQ-Y']);
    assert.deepEqual(JSON.parse(JSON.stringify(s.entries.get('A'))), JSON.parse(JSON.stringify(live.entries.get('A'))));
    assert.equal(s.refusals.size, 0);
  }
});

// Old recordings have no prompt coverage line and reach JS unchanged.
test('seed replay of an unresolved old omission does not inherit carried covers', () => {
  const seed = {kind:'entries', carried_entries:[entry('A', {covers:['REQ-X']})], gates:[],
    replies:[{id:'A', text:JSON.stringify(entry()), criterion:'criterion'}]};
  const ctx = context({subjects:{acceptance:seed}}), s = state();
  ctx.seedAuthorLoop({phase:'acceptance', author:true, retryScopes:new Set()}, s);
  assert.deepEqual(JSON.parse(JSON.stringify(s.entries.get('A').covers || [])), []);
});

test('prompt coverage is a JSON array even when requirement strings need escaping', () => {
  const covers = ['quoted "id"', 'right ] bracket', 'line\nbreak', 'back\\slash'];
  const text = context().currentCoversText({covers});
  assert.ok(text.startsWith(`\nThis entry currently covers ${JSON.stringify(covers)}.`));
  assert.equal(text.split('\n').length, 2);
});

test('a refused draft never supplies coverage to the next live rewrite', async () => {
  const ctx = context();
  ctx.__archonValidateAcceptanceEntry = (_id, serialized) =>
    typeof JSON.parse(serialized).gap_permitted === 'boolean' ? '[]'
      : JSON.stringify([{text:'gap_permitted invalid', subject:'A'}]);
  const s = {entries:new Map([['A', entry('A', {covers:['REQ-P']})]]), retryIds:new Set(['A'])};
  const refused = await author(ctx, s, entry('A', {covers:['REQ-A'], gap_permitted:'invalid'}), 2);
  assert.equal(refused.result.status, 'failed');
  assert.deepEqual([...s.entries.get('A').covers], ['REQ-P']);
  const rewritten = await author(ctx, s, entry(), 3);
  assert.equal(rewritten.result.status, 'accepted');
  assert.deepEqual([...s.entries.get('A').covers], ['REQ-P']);
});
