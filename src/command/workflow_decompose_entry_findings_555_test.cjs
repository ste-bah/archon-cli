// #555: an acceptance entry's author prompt carries only that entry's own
// gate findings (current, and earlier ones not to regress) and the
// phase-level findings no entry owns, labelled as such. It never carries a
// sibling's finding: not in the shared part of the prompt, not in the
// earlier-attempts history, not in the named-file notes, and not when the
// loop starts from a frozen stage's findings or from a phase seed.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js',
  'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true}, gateEnvelope:{policy_findings:[]}});
const finding = (subject, text, extra = {}) => ({text, subject, remediation_scope:'candidate_artifact', ...extra});
const gate = (...findings) => () => ({postcondition:{satisfied:false}, gateEnvelope:{policy_findings:findings}});
const owed = req => finding('SUP', `check 'SUP-${req}': PRD requirement ${req} is covered by no acceptance check; author supplementary check SUP-${req} with covers ["${req}"]: text of ${req}`);
const requirements = {'REQ-1':'text of REQ-1', 'REQ-2':'text of REQ-2'};

function context(phaseSeed = null) {
  const ctx = {args:{acceptanceCriteria:{A:'a', B:'b', C:'c'}, prdRequirementTexts:requirements, authorMaxParallelism:1,
    gateMode:'enforce', taskRoot:'/tasks', ...(phaseSeed ? {phaseSeed} : {})},
    __archonValidateAcceptanceEntry: () => '[]',
    __archonHost: (kind, payload) => { throw new Error(`${kind}: ${JSON.parse(payload).reason}`); }};
  // The named-file prefetch reports the notes it was given, so a test can
  // see whether a sibling's finding reached them.
  const notes = [];
  ctx.__archonNamedFiles = (_entry, given) => { notes.push(JSON.parse(given).join('\n')); return '[]'; };
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  if (phaseSeed) ctx.applySeedOrdinals();
  return {ctx, notes};
}

function host(gates) {
  const seen = {authored:[], prompts:[], stdins:[]};
  const w = {
    agent: async (_, options) => {
      if (seen.authored.length > 40) throw new Error('bounded test exhausted');
      const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
      seen.authored.push(id);
      seen.prompts.push({id, task:options.task});
      return {status:'accepted', stopReason:'end_turn',
        content:JSON.stringify({id, version:seen.authored.filter(x => x === id).length, criterion:''})};
    },
    hostCommand: async (_, {stdin}) => {
      seen.stdins.push(stdin);
      if (seen.stdins.length > gates.length) throw new Error('unexpected freeze');
      return gates[seen.stdins.length - 1]();
    },
    pause: async () => ({resumed:true}),
    checkpoint: async () => ({}),
  };
  return {seen, w};
}

const policy = ctx => ({phase:'acceptance', prompt:() => 'author', author:ctx.authorAcceptanceEntries,
  capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])});

async function run(gates) {
  const {ctx, notes} = context();
  const {seen, w} = host(gates);
  await ctx.authorCandidate(w, policy(ctx));
  return {...seen, notes, prompt:(id, nth = 1) => seen.prompts.filter(p => p.id === id)[nth - 1]?.task};
}

// The shared part of an entry prompt: everything before its own section.
const shared = task => task.slice(0, task.indexOf('\nAuthor ONLY entry'));

async function anEntryPromptCarriesNoSiblingFinding() {
  const out = await run([gate(finding('A', "check 'A' was refuted: alpha-defect"), finding('B', "check 'B' was refuted: beta-defect")), clean]);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A', 'B']);
  const a = out.prompt('A', 2), b = out.prompt('B', 2);
  assert.match(a, /open in the latest gate result:\n- check 'A' was refuted: alpha-defect/, 'A keeps its own finding');
  assert.doesNotMatch(a, /beta-defect/, "A's prompt never carries B's finding");
  assert.match(b, /open in the latest gate result:\n- check 'B' was refuted: beta-defect/);
  assert.doesNotMatch(b, /alpha-defect/, "B's prompt never carries A's finding");
  for (const task of [a, b]) assert.doesNotMatch(task, /Repair these exact authoritative findings/, 'no whole-round repair list');
  assert.ok(out.notes.slice(3).every((text, index) => !text.includes(index === 0 ? 'beta-defect' : 'alpha-defect')),
    'the named-file notes carry no sibling finding');
}

async function aSupplementaryEntryGetsOnlyItsOwnFinding() {
  const out = await run([gate(owed('REQ-1'), owed('REQ-2'), finding('A', "check 'A' was refuted: alpha-defect")), clean]);
  assert.deepEqual(out.authored.slice(3).sort(), ['A', 'SUP-REQ-1', 'SUP-REQ-2']);
  const sup1 = out.prompt('SUP-REQ-1'), sup2 = out.prompt('SUP-REQ-2'), a = out.prompt('A', 2);
  assert.match(sup1, /check 'SUP-REQ-1': PRD requirement REQ-1 is covered by no acceptance check/);
  assert.match(sup2, /check 'SUP-REQ-2': PRD requirement REQ-2 is covered by no acceptance check/);
  // A completed sibling may be listed as context (its record line); its finding never is.
  assert.doesNotMatch(sup1, /check 'SUP-REQ-2'|REQ-2 is covered|alpha-defect/, 'SUP-REQ-1 reads no sibling finding');
  assert.doesNotMatch(sup2, /check 'SUP-REQ-1'|REQ-1 is covered|alpha-defect/, 'SUP-REQ-2 reads no sibling finding');
  assert.doesNotMatch(a, /SUP-REQ|covered by no acceptance check/, 'an entry never reads an owed supplementary finding');
}

async function aPhaseLevelFindingIsLabelledNotARepairList() {
  const whole = 'candidate artifact was refused: the contract as a whole proves nothing';
  const out = await run([gate(finding('acceptance', whole), finding('A', "check 'A' was refuted: alpha-defect")), clean]);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A', 'B', 'C'], 'an unattributable finding still re-authors every entry');
  for (const id of ['A', 'B', 'C']) {
    const task = out.prompt(id, 2);
    assert.match(task, /Phase-level findings of the latest gate result name no single entry[^\n]*:\n- candidate artifact was refused: the contract as a whole/,
      `${id} reads the phase-level finding under its own label`);
    assert.doesNotMatch(shared(task), /contract as a whole/, 'not in the shared repair list');
    assert.doesNotMatch(/open in the latest gate result:\n((?:- [^\n]*\n?)*)/.exec(task)?.[1] || '', /contract as a whole/,
      "not as the entry's own finding");
  }
  assert.doesNotMatch(out.prompt('B', 2), /alpha-defect/, "B still never reads A's finding");
}

async function theHistoryBlockHasNoSiblingFinding() {
  const out = await run([gate(finding('A', "check 'A': alpha-one"), finding('B', "check 'B': beta-one")),
    gate(finding('A', "check 'A': alpha-two"), finding('B', "check 'B': beta-two")), clean]);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A', 'B', 'A', 'B']);
  const a = out.prompt('A', 3);
  assert.match(a, /Do not regress them:\n- check 'A': alpha-one/, "A's own earlier finding is history");
  assert.doesNotMatch(a, /beta-/, "A's history never carries B's findings");
  assert.doesNotMatch(a, /Earlier attempts in this phase/, 'no whole-round history block');
}

async function noRungTellsAnEntryToRepairOthers() {
  const out = await run([1, 2, 3, 4, 5].map(n => gate(finding('A', `check 'A': defect-${n}`), finding('B', `check 'B': other-${n}`))).concat([clean]));
  const tasks = out.prompts.map(p => p.task);
  assert.ok(tasks.some(task => /Repair operation r4/.test(task)), 'the loop reaches rung 4');
  for (const task of tasks) {
    assert.doesNotMatch(task, /deferred while independent subjects settle/, 'an entry is never told it waits on others');
    assert.doesNotMatch(task, /Prioritize this group first/, 'no group of the whole round');
  }
  for (const {id, task} of out.prompts) assert.doesNotMatch(task, id === 'A' ? /other-\d/ : /defect-\d/);
}

async function aFrozenStageRepairStartsEachEntryWithItsOwnFinding() {
  const {ctx} = context();
  const {seen, w} = host([clean]);
  const contract = {acceptance:['A', 'B', 'C'].map(id => ({id, criterion:id.toLowerCase(), version:0})), supplementary:[]};
  const at = {source_path:'/tasks/acceptance-contract.json'};
  await ctx.repairFrozenStage(w, {gateEnvelope:{report:JSON.stringify(contract)}},
    [finding('A', "check 'A' was refuted: alpha-defect", at), finding('B', "check 'B' was refuted: beta-defect", at)]);
  assert.deepEqual(seen.authored, ['A', 'B']);
  const [a, b] = seen.prompts.map(p => p.task);
  assert.match(a, /alpha-defect/);
  assert.doesNotMatch(a, /beta-defect/);
  assert.match(b, /beta-defect/);
  assert.doesNotMatch(b, /alpha-defect/);
}

async function aSeededLoopStartsEachEntryWithItsOwnFinding() {
  const entry = id => ({id, criterion:id.toLowerCase(), check:{kind:'command', command:`t ${id}`, cwd:'project_root'},
    gap_permitted:false, covers:[], judgment:{verdict:'accepted', counterexample:'', reason:'', host_call_id:''}});
  const candidate = JSON.stringify({entries:['A', 'B', 'C'].map(entry), supplementary:[]});
  const {ctx} = context({transition_index:0, author_ordinals:{acceptance:25}, pause_ordinals:{acceptance:2},
    subjects:{acceptance:{kind:'entries', candidate, replies:[], invalid:{C:['acceptance entry C: gamma-invalid']}, carried:3,
      gates:[{call_id:'g', published:false, findings:[finding('A', "check 'A' was refuted: alpha-defect"),
        finding('B', "check 'B' was refuted: beta-defect")]}]}}});
  const {seen, w} = host([clean]);
  await ctx.authorCandidate(w, policy(ctx));
  assert.deepEqual(seen.authored, ['A', 'B', 'C']);
  const [a, b, c] = seen.prompts.map(p => p.task);
  assert.match(a, /alpha-defect/);
  assert.doesNotMatch(a, /beta-defect|gamma-invalid/);
  assert.match(b, /beta-defect/);
  assert.doesNotMatch(b, /alpha-defect|gamma-invalid/);
  assert.match(c, /gamma-invalid/);
  assert.doesNotMatch(c, /alpha-defect|beta-defect/);
}

// A stale freeze names no entry and is the loop's to rebuild: it is no
// phase-level finding for an author, and the entry it sends back reads only
// its own finding.
async function aStaleFreezeIsNoAuthorsFinding() {
  const stale = () => ({postcondition:{satisfied:false}, gateEnvelope:{
    report:{gate_report:'prepared', current_acceptance_contract:{acceptance:[{id:'B', version:'published', criterion:'b'}], supplementary:[]}},
    policy_findings:[finding('acceptance', 'the acceptance contract changed after this freeze was prepared; refresh it',
      {deterministic_defect:{code:'stale_acceptance_contract', subject:'acceptance', provenance:'host'}}),
    finding('B', "check 'B' was refuted: beta-defect"), finding('C', "check 'C' was refuted: gamma-defect")]}});
  const out = await run([stale, clean]);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'C']);
  const c = out.prompt('C', 2);
  assert.match(c, /open in the latest gate result:\n- check 'C' was refuted: gamma-defect/);
  assert.doesNotMatch(c, /contract changed after this freeze|Phase-level|beta-defect/);
}

const tests = [
  ['an entry prompt carries no sibling finding', anEntryPromptCarriesNoSiblingFinding],
  ['a supplementary entry gets only its own finding', aSupplementaryEntryGetsOnlyItsOwnFinding],
  ['a phase-level finding is labelled, not a repair list', aPhaseLevelFindingIsLabelledNotARepairList],
  ['the history block has no sibling finding', theHistoryBlockHasNoSiblingFinding],
  ['no rung tells an entry to repair others', noRungTellsAnEntryToRepairOthers],
  ['a frozen-stage repair starts each entry with its own finding', aFrozenStageRepairStartsEachEntryWithItsOwnFinding],
  ['a seeded loop starts each entry with its own finding', aSeededLoopStartsEachEntryWithItsOwnFinding],
  ['a stale freeze is no author\'s finding', aStaleFreezeIsNoAuthorsFinding],
];
module.exports = tests;
if (require.main === module) (async () => {
  let failed = 0;
  for (const [name, test] of tests) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (e) { failed++; console.error(`FAIL ${name}: ${e.message.slice(0, 600)}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
