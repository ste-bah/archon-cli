// Issue 360: an author loop seeded after a runtime upgrade re-authors only the
// entries that fail or were refuted, then runs the gate. Run with node.
// ARCHON_SEED_SAMPLE may name a derived seed (`{subjects}` JSON) and
// ARCHON_SEED_CRITERIA its criteria, to report what that seed re-authors.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js', 'workflow_decompose_v1_seed.js']
  .map((name) => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');

function context(criteria, phaseSeed, prdRequirementTexts = {}) {
  // Issue 357's native entry validator; its rules are tested by shape_tests.
  const ctx = { args: { acceptanceCriteria: criteria, prdRequirementTexts, gateMode: 'enforce', authorMaxParallelism: 2, phaseSeed },
    __archonValidateAcceptanceEntry: () => '[]' };
  vm.createContext(withAuthorContext(ctx));
  vm.runInContext(source, ctx);
  if (phaseSeed) ctx.applySeedOrdinals();
  return ctx;
}
const entry = (id, version = 0) => ({ id, criterion: id.toLowerCase(), check: { kind: 'command', command: `t ${id} ${version}`, cwd: 'project_root' },
  gap_permitted: false, covers: [], judgment: { verdict: 'accepted', counterexample: '', reason: '', host_call_id: '' } });
const finding = (text, subject = 'acceptance') => ({ text, subject, remediation_scope: 'candidate_artifact' });
const owedText = (req) => `check 'SUP-${req}': PRD requirement ${req} is covered by no acceptance check; author supplementary check SUP-${req}: the ${req} text`;
const acceptancePolicy = (ctx) => ({ phase: 'acceptance', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']),
  shadowScopes: new Set(), author: (w, prompt, round, state) => ctx.authorAcceptanceEntries(w, prompt, round, state) });
const clean = (call) => ({ publicationReceipt: { call_id: call }, postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] },
  result: { data: { publicationReceipt: { call_id: call } } } });

// Records every agent and gate call; every gate accepts.
function host() {
  const seen = { agents: [], gates: [] };
  return { seen, w: {
    agent: async (id, options) => {
      seen.agents.push(id);
      const [, entryId] = /^acceptance-author-(.+)-\d+$/.exec(id) || [];
      return { status: 'accepted', stopReason: 'end_turn', content: entryId ? JSON.stringify(entry(entryId, 1)) : `artifact ${id}`, task: options.task };
    },
    hostCommand: async (capability, request) => { seen.gates.push({ capability, stdin: request.stdin }); return clean(`gate-${seen.gates.length}`); },
    pause: async (id) => { throw new Error(`unexpected pause ${id}`); }
  } };
}

async function seeded(subjects, criteria, ordinals = { acceptance: 25 }, requirementTexts = {}) {
  const ctx = context(criteria, { transition_index: 0, author_ordinals: ordinals, pause_ordinals: { acceptance: 2 }, subjects }, requirementTexts);
  const { seen, w } = host();
  const outcome = await ctx.authorCandidate(w, acceptancePolicy(ctx));
  return { ctx, seen, outcome };
}

async function refutedEntryOnly() {
  const candidate = JSON.stringify({ entries: [entry('A'), entry('B'), entry('C')], supplementary: [] });
  const { seen } = await seeded({ acceptance: { kind: 'entries', candidate, replies: [], invalid: {}, carried: 3,
    gates: [{ call_id: 'g', published: false, findings: [finding("check 'B' was refuted by the host judge; reason: weak", 'B')] }] } },
  { A: 'a', B: 'b', C: 'c' });
  assert.deepEqual(seen.agents, ['acceptance-author-B-28'], 'only the refuted entry, in a round after every old one');
  const submitted = JSON.parse(seen.gates[0].stdin);
  assert.deepEqual(submitted.entries.map((e) => e.check.command), ['t A 0', 't B 1', 't C 0']);
}

// A gate asked the same candidate again answers from its old record, so a
// reply after it that repeats the judged entry is no repair.
async function unchangedReplyIsNoRepair() {
  const candidate = JSON.stringify({ entries: [entry('A'), entry('B')], supplementary: [] });
  const { seen } = await seeded({ acceptance: { kind: 'entries', candidate, invalid: {}, carried: 2,
    replies: [{ call_id: 'acceptance-author-B-13', id: 'B', text: JSON.stringify(entry('B')) }],
    gates: [{ call_id: 'g', published: false, findings: [finding("check 'B' was refuted by the host judge; reason: weak", 'B')] }] } },
  { A: 'a', B: 'b' });
  assert.deepEqual(seen.agents, ['acceptance-author-B-28']);
}

// The last gate refused the candidate with a finding that names no entry. The
// live loop then re-authors every entry; the seed does the same for each entry
// not re-authored (changed) since, and never re-submits the refused candidate.
async function unattributableRefusalReauthorsAsTheLoopDoes() {
  const candidate = JSON.stringify({ entries: [entry('A'), entry('B'), entry('C')], supplementary: [] });
  const whole = finding('candidate artifact was refused: the contract as a whole proves nothing about the PRD');
  const subjects = (replies, findings = [whole]) => ({ acceptance: { kind: 'entries', candidate, replies, invalid: {}, carried: 3,
    gates: [{ call_id: 'g', published: false, findings }] } });
  const criteria = { A: 'a', B: 'b', C: 'c' };
  // No reply since the refusal: every entry, and a candidate unlike the refused one.
  let run = await seeded(subjects([]), criteria);
  assert.deepEqual(run.seen.agents, ['acceptance-author-A-28', 'acceptance-author-B-28', 'acceptance-author-C-28']);
  assert.notEqual(run.seen.gates[0].stdin, candidate, 'the refused candidate is never re-submitted unchanged');
  // A changed reply since is that round's work: kept, the others authored.
  run = await seeded(subjects([{ call_id: 'acceptance-author-B-25', id: 'B', text: JSON.stringify(entry('B', 7)) }]), criteria);
  assert.deepEqual(run.seen.agents, ['acceptance-author-A-28', 'acceptance-author-C-28']);
  assert.deepEqual(JSON.parse(run.seen.gates[0].stdin).entries.map((e) => e.check.command), ['t A 1', 't B 7', 't C 1']);
  // A reply that repeats the refused entry repairs nothing.
  run = await seeded(subjects([{ call_id: 'acceptance-author-B-25', id: 'B', text: JSON.stringify(entry('B')) }]), criteria);
  assert.deepEqual(run.seen.agents, ['acceptance-author-A-28', 'acceptance-author-B-28', 'acceptance-author-C-28']);
  // Beside a finding that names one entry, the unattributable one still sends back all.
  run = await seeded(subjects([], [finding("check 'B' was refuted by the host judge; reason: weak", 'B'), whole]), criteria);
  assert.deepEqual(run.seen.agents, ['acceptance-author-A-28', 'acceptance-author-B-28', 'acceptance-author-C-28']);
  // An unattributable finding of an EARLIER gate only registers owed checks.
  const gates = [{ call_id: 'g0', published: false, findings: [whole] },
    { call_id: 'g', published: false, findings: [finding("check 'B' was refuted by the host judge; reason: weak", 'B')] }];
  run = await seeded({ acceptance: { kind: 'entries', candidate, replies: [], invalid: {}, carried: 3, gates } }, criteria);
  assert.deepEqual(run.seen.agents, ['acceptance-author-B-28']);
}

async function invalidEntryOnly() {
  const candidate = JSON.stringify({ entries: [entry('A'), entry('B')], supplementary: [] });
  const { seen } = await seeded({ acceptance: { kind: 'entries', candidate, replies: [], carried: 2,
    invalid: { A: ["acceptance entry 'A' was refused: entry/criterion is missing"] },
    gates: [{ call_id: 'g', published: true, findings: [] }] } }, { A: 'a', B: 'b' });
  assert.deepEqual(seen.agents, ['acceptance-author-A-28']);
}

async function changedSupplementaryCriterionIsReauthored() {
  const changed = { ...entry('SUP-REQ-1'), criterion: 'old requirement text', covers: ['REQ-1'] };
  const unchanged = { ...entry('SUP-REQ-2'), criterion: 'unchanged requirement text', covers: ['REQ-2'] };
  const candidate = JSON.stringify({ entries: [entry('A')], supplementary: [changed, unchanged] });
  const ctx = context({ A: 'a' }, { transition_index: 0, author_ordinals: {}, pause_ordinals: {}, subjects: {} },
    { 'REQ-1': 'current requirement text', 'REQ-2': 'unchanged requirement text' });
  const state = { entries: new Map() };
  ctx.seedEntries({ candidate, replies: [], invalid: {}, gates: [] },
    { phase: 'acceptance', author: true, retryScopes: new Set(['candidate_artifact']) }, state);
  assert.deepEqual([...state.retryIds], ['SUP-REQ-1'], 'changed host text is re-authored while unchanged text is carried');
}

// The last gate refused the whole candidate by a pointer into it: the entry
// the pointer names was repaired since, so nothing is authored and the gate runs.
async function pointerRefusalRepairedSince() {
  const sup = entry('SUP-REQ-1');
  delete sup.criterion;
  const candidate = JSON.stringify({ entries: [entry('A')], supplementary: [sup] });
  const gates = [{ call_id: 'g1', published: false, findings: [finding(owedText('REQ-1'), 'SUP-REQ-1')] },
    { call_id: 'g2', published: false, findings: [finding('candidate artifact was refused: supplementary/0/criterion is missing or has an invalid type or value')] }];
  const repaired = entry('SUP-REQ-1', 2);
  repaired.gap_permitted = true;
  const subjects = (replies) => ({ acceptance: { kind: 'entries', candidate, gates, replies, invalid: {}, carried: 2 } });
  let run = await seeded(subjects([{ call_id: 'acceptance-author-SUP-REQ-1-25', id: 'SUP-REQ-1', text: JSON.stringify(repaired) }]), { A: 'a' });
  assert.deepEqual(run.seen.agents, [], 'carried, repaired after the refusal');
  const submitted = JSON.parse(run.seen.gates[0].stdin);
  assert.equal(submitted.supplementary[0].check.command, 't SUP-REQ-1 2');
  assert.deepEqual(submitted.supplementary[0].covers, ['REQ-1'], 'host-owned fields set as the author step sets them');
  assert.equal(submitted.supplementary[0].gap_permitted, false);
  // Not repaired since: the pointer names the one entry to author again.
  run = await seeded(subjects([]), { A: 'a' });
  assert.deepEqual(run.seen.agents, ['acceptance-author-SUP-REQ-1-28']);
}

async function neverFrozen() {
  const { seen } = await seeded({ acceptance: { kind: 'entries', gates: [], replies: [
    { call_id: 'acceptance-author-A-4', id: 'A', text: JSON.stringify(entry('A')) }], invalid: {}, carried: 1 } }, { A: 'a', B: 'b' }, { acceptance: 4 });
  assert.deepEqual(seen.agents, ['acceptance-author-B-7'], 'the missing entry alone');
  assert.equal(seen.gates.length, 1);
}

async function artifactSeeds() {
  const ctx = context({}, { transition_index: 0, author_ordinals: { skeleton: 3, 'body-T': 5 }, pause_ordinals: {}, subjects: {
    skeleton: { kind: 'artifact', candidate: 'kept skeleton', candidate_call: 'skeleton-author-3', gate: { call_id: 'g', published: true, findings: [] } },
    'body-T': { kind: 'artifact', candidate: 'old body', candidate_call: 'body-T-author-5',
      gate: { call_id: 'g2', published: false, findings: [finding('observation missing', 'T')] } } } });
  let { seen, w } = host();
  await ctx.authorCandidate(w, { phase: 'skeleton', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']) });
  assert.deepEqual(seen.agents, [], 'a clean candidate goes to its gate as it is');
  assert.equal(seen.gates[0].stdin, 'kept skeleton');
  ({ seen, w } = host());
  const prompts = [];
  w.agent = async (id, options) => { seen.agents.push(id); prompts.push(options.task); return { status: 'accepted', stopReason: 'end_turn', content: 'new body' }; };
  await ctx.authorCandidate(w, { phase: 'body-T', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']) });
  assert.deepEqual(seen.agents, ['body-T-author-6']);
  assert.match(prompts[0], /observation missing/);
  // A subject re-opened later in the run is not seeded twice.
  ({ seen, w } = host());
  await ctx.authorCandidate(w, { phase: 'skeleton', prompt: () => 'author', retryScopes: new Set(['candidate_artifact']) });
  assert.deepEqual(seen.agents, ['skeleton-author-4']);
}

async function pausesContinue() {
  const ctx = context({ A: 'a' }, { transition_index: 0, author_ordinals: {}, pause_ordinals: { acceptance: 2, 'set-gates': 1 }, subjects: {} });
  const ids = [];
  await assert.rejects(ctx.pauseLoop({ pause: async (id) => { ids.push(id); throw new Error('paused'); } }, 'set-gates', {}), /paused/);
  assert.deepEqual(ids, ['pause-set-gates-2'], 'an old pause is never passed as taken');
}

async function sample() {
  const path = process.env.ARCHON_SEED_SAMPLE;
  if (!path) return;
  const subjects = JSON.parse(fs.readFileSync(path, 'utf8'));
  const criteria = JSON.parse(fs.readFileSync(process.env.ARCHON_SEED_CRITERIA, 'utf8'));
  const { seen } = await seeded(subjects, criteria);
  const submitted = JSON.parse(seen.gates[0].stdin);
  console.log(`sample: re-authored=${seen.agents.length} [${seen.agents.join(', ')}] gates=${seen.gates.length} entries=${submitted.entries.length} supplementary=${submitted.supplementary.length}`);
}

(async () => {
  for (const test of [refutedEntryOnly, unchangedReplyIsNoRepair, unattributableRefusalReauthorsAsTheLoopDoes, invalidEntryOnly,
    changedSupplementaryCriterionIsReauthored, pointerRefusalRepairedSince, neverFrozen, artifactSeeds, pausesContinue, sample]) {
    await test();
    console.log(`ok ${test.name}`);
  }
})().catch((error) => { console.error(error); process.exit(1); });
