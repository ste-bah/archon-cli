const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_seed.js', 'workflow_decompose_v1_context.js', 'workflow_decompose_v1_progress.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
function context() {
  const ctx = {args:{prdRequirementTexts:Object.fromEntries(
    ['DONE-1', 'NFR-001', 'G-X-001', 'TF-X-001', 'RD-X-001'].map(id => [id, `text ${id}`]))}};
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  return ctx;
}
function wireFinding(code, subject, text = '') {
  return {text, subject, remediation_scope:'candidate_artifact',
    deterministic_defect:{provenance:'host_validator', code, subject, location:'covers', stage:'contracts'}};
}
function finding(id) {
  return wireFinding('uncovered_requirement', `SUP-${id}`);
}
test('one supplementary grammar accepts every class and strips only numeric revision suffixes', () => {
  const ctx = context();
  for (const id of Object.keys(ctx.args.prdRequirementTexts)) {
    assert.equal(ctx.supplementaryRequirement(`SUP-${id}`), id);
    assert.equal(ctx.supplementaryRequirement(`SUP-${id}-R23`), id);
    assert.equal(ctx.currentRequirementText(id), `text ${id}`);
    const retry = ctx.acceptanceRepairIds([finding(id)], new Set(), false);
    assert.deepEqual([...retry], [`SUP-${id}`]);
  }
  for (const id of ['', 'SUP-', 'SUP-a/b', 'SUP-a b', 'SUP-a\n', 'SUP-aé', null, 42]) assert.equal(ctx.supplementaryRequirement(id), null);
  assert.equal(ctx.supplementaryRequirement('SUP-NFR-001-Rx'), 'NFR-001-Rx');
  ctx.acceptanceRepairIds([finding('AC-X-001')], new Set(['AC-X-001']), false);
  assert.equal(ctx.owedSupplementary().has('SUP-AC-X-001'), false);
  ctx.acceptanceRepairIds([finding('UNKNOWN-001')], new Set(), false);
  assert.equal(ctx.owedSupplementary().has('SUP-UNKNOWN-001'), false);
});
test('an existing covering entry prevents a new supplementary debt', () => {
  const ctx = context(), f = finding('DONE-1');
  const entry = {id:'AC-X-001', covers:['DONE-1']};
  for (const candidate of [{entries:[entry]}, {acceptance:[entry]}, {supplementary:[entry]}, {entries:[{...entry, covers:[' DONE-1 ']}]}]) {
    const retry = ctx.acceptanceRepairIds([f], new Set(), false, candidate);
    assert.deepEqual([...retry], []);
    assert.equal(ctx.owedSupplementary().has(f.subject), false);
  }
  assert.deepEqual([...ctx.acceptanceRepairIds([f], new Set(), false, null, new Map([[entry.id, entry]]))], []);
});
test('capability origins retain debt through early refusals and prune on coverage results', () => {
  const ctx = context();
  const repair = (fs, origin, candidate = null) => ctx.acceptanceRepairIds(fs, new Set(), false, candidate, null, origin);
  repair([finding('DONE-1')], 'freeze-acceptance');
  repair([finding('NFR-001')], 'freeze-skeleton');
  assert.equal(ctx.owedSupplementary().get('SUP-DONE-1').origin, 'freeze-acceptance');
  repair([], 'freeze-acceptance');
  repair([wireFinding('duplicate_check', 'AC-X-001')], 'freeze-acceptance');
  assert.equal(ctx.owedSupplementary().size, 2);
  repair([finding('G-X-001')], 'freeze-acceptance');
  assert.deepEqual([...ctx.owedSupplementary().keys()], ['SUP-NFR-001', 'SUP-G-X-001']);
  repair([], 'freeze-skeleton', {entries:[{id:'AC-X-001', covers:['NFR-001']}]});
  assert.deepEqual([...ctx.owedSupplementary().keys()], ['SUP-G-X-001']);
});
test('an owed supplementary entry cannot deduplicate itself', () => {
  for (const held of [false, true]) {
    const ctx = context(), f = finding('DONE-1'), entry = {id:f.subject, covers:['DONE-1']};
    ctx.acceptanceRepairIds([f], new Set(), false, held ? null : {supplementary:[entry]}, held ? new Map([[entry.id, entry]]) : null, 'freeze-acceptance');
    assert.equal(ctx.owedSupplementary().has(f.subject), true);
  }
});
test('one gate batch retains all omissions while attributing individual findings', () => {
  const ctx = context(), findings = [finding('DONE-1'), finding('NFR-001')];
  const result = ctx.attributeEntryFindings(findings, new Set(), false, {}, new Map());
  assert.deepEqual([...ctx.owedSupplementary().keys()], ['SUP-DONE-1', 'SUP-NFR-001']);
  assert.equal(result.own.size, 2);
});

test('text-only historical findings use the shared grammar and current text', () => {
  const ctx = context();
  const f = {subject:"", remediation_scope:"candidate_artifact", text:"check 'SUP-DONE-1-R2': PRD requirement DONE-1 is covered by no acceptance check; author it: old text"};
  assert.deepEqual([...ctx.acceptanceRepairIds([f], new Set(), false)], ['SUP-DONE-1-R2']);
  assert.equal(ctx.owedSupplementary().get('SUP-DONE-1-R2').text, 'text DONE-1');
  assert.equal(ctx.acceptanceRepairIds([{text:f.text.replace('requirement DONE-1', 'requirement NFR-001')}], new Set(), false), null);
});
test('a later covering check clears a still-reported debt without authoring another check', () => {
  const ctx = context(), f = finding('DONE-1');
  ctx.acceptanceRepairIds([f], new Set(), false);
  const covered = {entries:[{id:'AC-X-001', covers:['DONE-1']}]};
  assert.deepEqual([...ctx.acceptanceRepairIds([f], new Set(), false, covered)], []);
  assert.equal(ctx.owedSupplementary().has(f.subject), false);
});

test('caller capability is retained across sibling results', () => {
  const ctx = context(), f = finding('DONE-1');
  ctx.attributeEntryFindings([f], new Set(), false, {}, new Map(), 'freeze-acceptance');
  assert.equal(ctx.owedSupplementary().get(f.subject).origin, 'freeze-acceptance');
  ctx.attributeEntryFindings([finding('NFR-001')], new Set(), false, {}, new Map(), 'freeze-skeleton');
  assert.equal(ctx.owedSupplementary().has(f.subject), true);
  ctx.attributeEntryFindings([finding('G-X-001')], new Set(), false, {}, new Map(), 'freeze-acceptance');
  assert.equal(ctx.owedSupplementary().has(f.subject), false);
});
test('coverable catalogue includes minted ids and exact texts with bounded file fallback', () => {
  const ctx = context(), files = new Map();
  ctx.__archonAuthorContext = (_, text) => {
    const sha256 = require('node:crypto').createHash('sha256').update(text).digest('hex');
    const path = `/context/${sha256}.json`;
    files.set(path, text);
    return JSON.stringify({path, sha256});
  };
  const head = 'Coverable IDs and exact texts:';
  const small = ctx.catalogueText(ctx.args.prdRequirementTexts, 'DONE-1', [], head, 5120, "ids");
  assert(small.includes('DONE-1'));
  assert(small.includes('text DONE-1'));
  const large = Object.fromEntries(Array.from({length:100}, (_, i) => [`DONE-${i}`, '界'.repeat(200)]));
  const bounded = ctx.catalogueText(large, 'DONE-1', [], head, 5120, "ids");
  assert(Buffer.byteLength(bounded) < 5600);
  assert(bounded.includes("100 ids;"));
  assert(source.includes('"All criterion IDs and text (for consistency):", CATALOGUE_SHARE)'));
  const file = bounded.match(/the exact catalogue is (\S+) \(sha256/)[1];
  assert.equal(files.get(file), JSON.stringify(large));
  assert(source.includes('catalogueText(args.prdRequirementTexts || {}, id, state.order, "Coverable IDs and exact texts:", COVERABLE_SHARE, "ids")'));
});

test('a stale publication result keeps the caller capability on new debt', () => {
  const ctx = context();
  ctx.refreshFromCurrentContract({entries:new Map()}, {current_acceptance_contract:null},
    [finding('DONE-1')], new Set(), {entries:[]}, 'freeze-acceptance');
  assert.equal(ctx.owedSupplementary().get('SUP-DONE-1').origin, 'freeze-acceptance');
});

test('legacy free-form supplementary entries resume without registering debt', () => {
  const ctx = context();
  const entry = {id:'SUP-FREE-001', covers:[]};
  const state = {entries:new Map()};
  ctx.seedEntries({carried_entries:[entry]}, {}, state);
  assert.equal(state.entries.get(entry.id), entry);
  assert.equal(ctx.owedSupplementary().size, 0);
  ctx.refreshFromCurrentContract(state, {current_acceptance_contract:{acceptance:[], supplementary:[entry]}}, [], new Set(), {entries:[]});
  assert.equal(ctx.owedSupplementary().size, 0);
});

test('frozen repair keeps a legacy free-form supplementary entry without debt', async () => {
  const ctx = context();
  const entry = {id:'SUP-FREE-001', covers:[]};
  ctx.frozenFindingOwner = () => 'acceptance';
  ctx.acceptanceStagePolicy = () => ({});
  ctx.authorAcceptanceEntries = async (_, __, ___, state) => {
    assert.equal(state.entries.get(entry.id).id, entry.id);
    return JSON.stringify({supplementary:[entry]});
  };
  ctx.authorCandidate = async (_, policy) => ({candidate:await policy.author({}, '', 1, {})});
  await ctx.repairFrozenStage({}, {gateEnvelope:{report:JSON.stringify({acceptance:[], supplementary:[entry]})}}, []);
  assert.equal(ctx.owedSupplementary().size, 0);
});

test('criteria catalogue between five and ten KiB remains whole', () => {
  const ctx = context();
  const criteria = {'AC-X-001':'x'.repeat(8000)};
  const whole = JSON.stringify(criteria);
  assert(Buffer.byteLength(whole) > 5120 && Buffer.byteLength(whole) < 10240);
  const shown = ctx.catalogueText(criteria, 'AC-X-001', []);
  assert(shown.endsWith(whole));
  assert(!shown.includes('nearest this entry'));
});
