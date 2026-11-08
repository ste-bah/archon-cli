// Issue 562: an attempt with fewer open findings than ever before, or with a
// subject at a later gate stage than ever before, is progress; a repeat, a
// trade or an oscillation still pauses.
// Runs the script from ARCHON_TEST_SCRIPT_ROOT (default: this directory).
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const files = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js'];
const source = files.map(file => fs.readFileSync(`${scriptRoot}/${file}`, 'utf8')).join('\n');
function freshContext() {
  const context = { args: { gateMode: 'enforce' } };
  vm.createContext(withAuthorContext(context));
  vm.runInContext(source, context);
  return context;
}
const plain = value => JSON.parse(JSON.stringify(value));

// Findings as the host writes them. `stage` is the envelope's gate_stage;
// undefined is a finding of the gate's own validators, or an envelope
// written before findings carried a stage.
const judged = (subject, stage, text = `judged ${subject}`, item) => ({ subject, text,
  remediation_scope: 'candidate_artifact', ...(stage ? { gate_stage: stage } : {}), ...(item ? { judged_item: item } : {}) });
const defect = (subject, code, location, stage) => ({ subject, text: `${code} ${subject} ${location}`,
  remediation_scope: 'candidate_artifact', ...(stage ? { gate_stage: stage } : {}),
  deterministic_defect: { provenance: 'host_validator', code, subject, location, stage: 'contracts' } });
const probe = subject => defect(subject, 'check_not_proven', 'check', 'probe');

// Records attempts; returns each attempt's progress flag and the reason the
// loop would stop after it.
function attempts(sets, context = freshContext()) {
  const progress = context.newProgress([]);
  const steps = sets.map((findings, index) => {
    const flag = context.recordAttempt(progress, index + 1, findings);
    context.openRepairEpisode(progress);
    return { flag, reason: context.stallReason(progress) };
  });
  return { progress, flags: steps.map(step => step.flag), reasons: steps.map(step => step.reason) };
}

// R13's eight acceptance freezes, read from their gate envelopes: each
// candidate_artifact finding as "subject kind [location]". J: refuted by the
// host judge; P: cannot pass as written (passability); cnp: check_not_proven
// (the pre-implementation probe); unc / ukc: uncovered_requirement /
// unknown_covered_requirement (the gate's validators). S- is SUP-REQ-.
const R13_FREEZES = [
  'AC-AHDM-002 J, AC-DL-002 J, AC-AHDM-001 cnp, AC-AHDM-003 cnp, AC-AHDM-004 cnp, AC-DL-003 cnp, AC-DL-004 cnp, AC-DL-006 cnp, AC-DL-007 cnp, S-AHDM-002 unc, S-AHDM-003 unc, S-AHDM-004 unc, S-AHDM-006 unc, S-AHDM-007 unc, S-AHDM-009 unc, S-AHDM-010 unc, S-AHDM-033 unc, S-DL-010 unc, S-DL-011 unc, S-DL-012 unc, S-DL-013 unc, S-DL-054 unc, S-DL-056 unc, S-DL-061 unc, S-DL-062 unc, S-DL-070 unc, S-DL-071 unc, S-DL-074 unc, S-DL-085 unc, S-DL-102 unc, S-DL-122 unc, S-DL-123 unc, S-DL-124 unc, S-DL-130 unc, S-DL-131 unc, S-DL-132 unc, S-DL-133 unc, AC-AHDM-001 ukc covers/0, AC-AHDM-002 ukc covers/0, AC-AHDM-003 ukc covers/0, AC-AHDM-004 ukc covers/9, AC-AHDM-004 ukc covers/10, AC-DL-001 ukc covers/0, AC-DL-002 ukc covers/0, AC-DL-003 ukc covers/4, AC-DL-003 ukc covers/5, AC-DL-004 ukc covers/0, AC-DL-005 ukc covers/0, AC-DL-006 ukc covers/0, AC-DL-006 ukc covers/16, AC-DL-007 ukc covers/0',
  'AC-AHDM-003 J, S-AHDM-003 J, S-AHDM-004 J, S-AHDM-009 J, S-DL-010 J, S-DL-012 J, S-DL-124 J, AC-AHDM-001 cnp, AC-AHDM-004 cnp, AC-DL-003 cnp, AC-DL-004 cnp, AC-DL-005 cnp, S-AHDM-006 cnp, S-AHDM-007 cnp, S-AHDM-033 cnp, S-DL-054 cnp, S-DL-123 cnp, S-AHDM-001 unc, S-AHDM-021 unc, S-AHDM-024 unc, S-AHDM-030 unc, S-AHDM-031 unc, S-AHDM-032 unc, S-BT-001 unc, S-BT-004 unc, S-DL-072 unc, S-DL-110 unc, S-DL-111 unc, AC-AHDM-002 ukc covers/4',
  'AC-AHDM-002 J, AC-DL-004 J, S-AHDM-033 J, AC-DL-005 cnp, S-DL-110 cnp, S-DL-130 cnp, S-AHDM-020 unc, S-AHDM-023 unc, S-DL-073 unc, S-DL-080 unc, S-DL-081 unc, S-DL-082 unc',
  'S-AHDM-020 J, S-DL-080 J, AC-AHDM-002 cnp',
  'AC-DL-003 P, AC-DL-004 P, S-DL-010 P, S-DL-054 P, S-DL-056 P, S-DL-061 P, S-DL-071 P, S-DL-072 P, S-DL-073 P, S-DL-082 P, S-DL-102 P, S-DL-110 P',
  'AC-DL-003 cnp, S-DL-073 cnp, S-DL-110 cnp, S-DL-002 unc, S-DL-003 unc, S-DL-004 unc, S-DL-005 unc',
  'AC-DL-003 J, S-DL-110 J, S-DL-005 cnp',
  'S-DL-110 cnp'
];
// The author calls of that loop: the freezes, and between them the calls
// that refused one entry each for its shape.
const R13_FREEZE_CALLS = [2, 6, 9, 11, 12, 13, 14, 15];
const R13_SHAPE_CALLS = [[1, 'AC-DL-001'], [3, 'AC-AHDM-001'], [4, 'AC-DL-003'], [5, 'S-AHDM-010'], [7, 'S-AHDM-024'],
  [8, 'S-DL-110'], [10, 'S-DL-081']];
const FULL_ID = id => id.replace(/^S-/, 'SUP-REQ-');

// `stamped`: the gate stage the host now writes on each finding (J judge,
// P passability, cnp probe); unstamped: the envelopes exactly as recorded.
function r13Findings(freeze, stamped) {
  return freeze.split(', ').map(row => {
    const [short, kind, location] = row.split(' ');
    const subject = FULL_ID(short);
    if (kind === 'J') return judged(subject, stamped ? 'judge' : undefined, `check '${subject}' was refuted by the host judge`);
    if (kind === 'P') return judged(subject, stamped ? 'passability' : undefined, `check '${subject}': cannot pass as written`);
    if (kind === 'cnp') return defect(subject, 'check_not_proven', 'check', stamped ? 'probe' : undefined);
    return defect(subject, kind === 'unc' ? 'uncovered_requirement' : 'unknown_covered_requirement',
      kind === 'unc' ? 'covers' : location);
  });
}

// Feeds the loop's calls in order, pausing as authorCandidate does.
async function driveR13(context, stamped) {
  const progress = context.newProgress([]), pauses = [];
  const w = { pause: async (id, evidence) => { pauses.push({ id, ...evidence }); } };
  const steps = [...R13_FREEZE_CALLS.map((call, index) => ({ call, findings: r13Findings(R13_FREEZES[index], stamped) })),
    ...R13_SHAPE_CALLS.map(([call, entryId]) => ({ call, refusal: { entryId: FULL_ID(entryId),
      malformedClass: 'parse-error:other:location-unknown',
      findings: [{ text: 'not json', deterministic_defect: { provenance: 'host_validator', code: 'invalid_json', subject: FULL_ID(entryId) } }] } }))]
    .sort((a, b) => a.call - b.call);
  for (const step of steps) {
    if (step.refusal) context.recordRepairs(progress, step.call, [step.refusal]);
    else { context.recordAttempt(progress, step.call, step.findings); context.openRepairEpisode(progress); }
    const stall = context.stallReason(progress);
    if (stall) await context.pauseAuthorLoop(w, 'acceptance', progress, stall, []);
  }
  return { progress, pauses };
}

async function r13FreezesNeverPause() {
  for (const stamped of [true, false]) {
    const { progress, pauses } = await driveR13(freshContext(), stamped);
    const judgedCalls = progress.history.filter(entry => entry.kind === 'judged');
    assert.deepEqual(plain(judgedCalls.map(entry => [entry.call, entry.stage, entry.findings])),
      [[2, 'contracts', 49], [6, 'contracts', 22], [9, 'contracts', 9], [11, 'contracts', 1], [12, 'passed', 0],
        [13, 'contracts', 7], [14, 'contracts', 1], [15, 'contracts', 1]], 'the fixture measures the live freezes');
    assert.deepEqual(pauses, [], `no pause in all eight freezes (stamped: ${stamped})`);
    assert.equal(progress.history.at(-1).progress, true, 'the last freeze has the fewest open findings');
    assert.equal(progress.bestOpen, 1);
  }
}

async function singleSubjectAdvancingThroughStagesDoesNotPause() {
  // Refuted twice by the judge, then the probe's finding, then passability.
  const { flags, reasons, progress } = attempts([[judged('AC-X-001', 'judge')], [judged('AC-X-001', 'judge', 'reworded')],
    [probe('AC-X-001')], [judged('AC-X-001', 'passability')]]);
  assert.deepEqual(flags, [true, false, true, true]);
  assert.deepEqual(reasons, [null, null, null, null]);
  assert.deepEqual(plain(progress.history.map(entry => entry.advanced || [])), [[], [], ['AC-X-001'], ['AC-X-001']]);
  // A subject that goes back to an earlier stage and returns is no credit.
  const back = attempts([[judged('AC-X-001', 'judge')], [judged('AC-X-001', 'passability')], [probe('AC-X-001')],
    [judged('AC-X-001', 'judge')], [judged('AC-X-001', 'passability')]]);
  assert.deepEqual(back.flags, [true, true, false, false, false]);
  assert.equal(back.reasons.at(-1), 'no_progress');
}

function sameStageRepeatedPauses() {
  for (const finding of [judged('AC-X-001', 'judge'), probe('AC-X-001'), judged('AC-X-001', 'passability'),
    judged('AC-X-001', 'critic', 'critic', 'REQ-1')]) {
    const { flags, reasons } = attempts([[finding], [finding], [finding], [finding]]);
    assert.deepEqual(flags, [true, false, false, false], `${finding.gate_stage}`);
    assert.deepEqual(reasons, [null, null, null, 'no_progress']);
  }
  // An advance opens a fresh window; three repeats after it pause.
  const { flags, reasons } = attempts([[judged('AC-X-001', 'judge')], [probe('AC-X-001')], [probe('AC-X-001')],
    [probe('AC-X-001')], [probe('AC-X-001')]]);
  assert.deepEqual(flags, [true, true, false, false, false]);
  assert.equal(reasons.at(-1), 'no_progress');
}

// Findings are not matched across attempts: a coverage failure traded from A
// to B, while A's passability refutation goes back to a judge refutation,
// keeps the count and raises A's earliest stage, yet A did not move on.
function tradedEarlyFailureWithARegressionIsNoAdvance() {
  const coverage = subject => defect(subject, 'uncovered_requirement', 'covers');
  const { flags, progress } = attempts([[coverage('AC-A'), judged('AC-A', 'passability'), judged('AC-B', 'judge')],
    [coverage('AC-B'), judged('AC-A', 'judge'), judged('AC-B', 'judge')]]);
  assert.deepEqual(flags, [true, false], 'a trade that regresses A is not progress');
  assert.equal(progress.history.at(-1).advanced, undefined);
  // The same subject with one finding moving judge, probe, passability still advances.
  const single = attempts([[judged('AC-A', 'judge')], [probe('AC-A')], [judged('AC-A', 'passability')]]);
  assert.deepEqual(single.flags, [true, true, true]);
  // Codex r2: B's coverage failure is only relocated and B regresses while A's
  // earliest rises and its furthest holds: not credited.
  const relocated = attempts([[coverage('AC-A'), judged('AC-A', 'passability'), judged('AC-B', 'judge')],
    [coverage('AC-B'), judged('AC-A', 'passability'), judged('AC-B', 'judge')]]);
  assert.deepEqual(relocated.flags, [true, false]);
  // Codex r3: the same invalid reference relocated within B (covers/0 to
  // covers/1) while A drops its own: B gains a finding without advancing.
  const unknown = (subject, index) => defect(subject, 'unknown_covered_requirement', `covers/${index}`);
  const moved = attempts([[unknown('AC-A', 0), judged('AC-A', 'passability'), unknown('AC-B', 0), judged('AC-B', 'judge')],
    [judged('AC-A', 'passability'), unknown('AC-B', 0), unknown('AC-B', 1), judged('AC-B', 'judge')]]);
  assert.deepEqual(moved.flags, [true, false]);
  assert.equal(moved.progress.history.at(-1).advanced, undefined);
  const swapped = attempts([[unknown('AC-A', 0), judged('AC-A', 'passability'), unknown('AC-B', 0), judged('AC-B', 'judge')],
    [judged('AC-A', 'passability'), unknown('AC-B', 1), judged('AC-B', 'judge')]]);
  // One finding fewer (four to three) is still `fewer` progress; only the stage
  // credit is refused, since B gained covers/1 without advancing.
  assert.equal(swapped.progress.history.at(-1).advanced, undefined, 'B gained covers/1 without advancing');
  assert.equal(swapped.progress.history.at(-1).open_findings, 3);
  // Two subjects: both advance, or one advances and the other holds: credited;
  // one advances while the other regresses: not credited.
  const two = (second) => attempts([[judged('AC-A', 'judge'), judged('AC-B', 'judge')], second]).flags;
  assert.deepEqual(two([probe('AC-A'), probe('AC-B')]), [true, true]);
  assert.deepEqual(two([probe('AC-A'), judged('AC-B', 'judge')]), [true, true]);
  assert.deepEqual(two([probe('AC-A'), coverage('AC-B')]), [true, false]);
  // A subject the previous attempt did not name blocks the credit.
  assert.deepEqual(two([probe('AC-A'), judged('AC-B', 'judge'), judged('AC-C', 'judge')]), [true, false]);
  // Dropping its early failure while its furthest finding holds is an advance.
  const kept = attempts([[coverage('AC-A'), judged('AC-A', 'passability')], [coverage('AC-A'), judged('AC-A', 'passability')],
    [judged('AC-A', 'passability')]]);
  assert.deepEqual(kept.flags, [true, false, true]);
}

const critic = (items, task = 'TASK-A-001') => items.map(item => ({ subject: task, gate_stage: 'critic', judged_item: `REQ-${item}`,
  text: `obligation REQ-${item} is claimed by ${task} but none is obliged to make it true`, remediation_scope: 'body' }));

async function bodyCriticFindingsShrinking() {
  const shrink = attempts([critic([1, 2, 3, 4, 5]), critic([1, 2, 3]), critic([1, 2]), critic([1])]);
  assert.deepEqual(shrink.flags, [true, true, true, true], 'five, three, two, one critic findings');
  assert.deepEqual(shrink.reasons, [null, null, null, null]);
  const repeat = attempts([critic([1, 2]), critic([1, 2]), critic([1, 2]), critic([1, 2])]);
  assert.deepEqual(repeat.flags, [true, false, false, false]);
  assert.equal(repeat.reasons.at(-1), 'no_progress');
  const trade = attempts([critic([1, 2]), critic([2, 3]), critic([3, 4]), critic([1, 4])]);
  assert.equal(trade.reasons.at(-1), 'no_progress', 'a trade at the same count pauses');
  const oscillate = attempts([critic([1, 2]), critic([1]), critic([1, 2]), critic([1]), critic([1, 2])]);
  assert.deepEqual(oscillate.flags, [true, true, false, false, false], 'only the first visit to one finding counts');
  assert.equal(oscillate.reasons.at(-1), 'no_progress');
  // The body loop itself: a body whose critic findings fall 5, 3, 2, 1, 0
  // lands, and is never deferred as pending.
  const context = freshContext();
  const rounds = [[1, 2, 3, 4, 5], [1, 2, 3], [1, 2], [1], []];
  let gates = 0;
  const result = await context.authorCandidate({
    agent: async () => ({ status: 'accepted', stopReason: 'end_turn', content: 'body' }),
    hostCommand: async () => ({ publicationReceipt: { call_id: `publication-${++gates}` }, postcondition: { satisfied: true },
      gateEnvelope: { policy_findings: critic(rounds[gates - 1]) } }),
    checkpoint: async () => {}, pause: async () => { throw new Error('unexpected pause'); }
  }, { phase: 'body-TASK-A-001', prompt: () => 'author', retryScopes: new Set(['body']), deferAuthoredPending: true });
  assert.equal(result.pendingAuthor, undefined, 'the shrinking body is not deferred');
  assert.equal(gates, 5);
}

function setGateTradeAtTheSameTotalPauses() {
  const set = subjects => subjects.map(subject => judged(subject, 'critic', `obligation ${subject} not established`));
  const { flags, reasons } = attempts([set(['O-1', 'O-2']), set(['O-2', 'O-3']), set(['O-3', 'O-4']), set(['O-1', 'O-4'])]);
  assert.deepEqual(flags, [true, false, false, false]);
  assert.equal(reasons.at(-1), 'no_progress');
  // Lint findings the gate's validators report trade the same way.
  const lint = attempts([[judged('TASK-1'), judged('TASK-2')], [judged('TASK-2'), judged('TASK-3')],
    [judged('TASK-3'), judged('TASK-4')], [judged('TASK-1'), judged('TASK-4')]]);
  assert.equal(lint.reasons.at(-1), 'no_progress');
}

// Same-binary resume: the host replays every recorded call back to the
// recorded pause verbatim, answers that the pause was already taken, and the
// script continues with a fresh window (pauseLoop). The ratchets are rebuilt
// from the replayed findings, so the resumed run reaches the same pause with
// the same evidence, and after it the ratchets the live run built still decide.
async function replayRebuildsTheRatchets() {
  const subjects = Array.from({ length: 30 }, (_, index) => `AC-Y-${String(index).padStart(3, '0')}`);
  const recorded = [critic([1, 2, 3]), [judged('AC-X-001', 'judge'), ...critic([1, 2])], [probe('AC-X-001'), ...critic([2, 3])],
    [probe('AC-X-001'), ...critic([1, 2]), ...subjects.map(subject => judged(`${subject}-${'x'.repeat(600)}`, 'judge'))]];
  const after = [[probe('AC-X-001'), ...critic([1])], []];
  const run = async (outcomes, resumed) => {
    const context = freshContext(), pauses = [];
    let gates = 0;
    const w = {
      agent: async () => ({ status: 'accepted', stopReason: 'end_turn', content: 'body' }),
      hostCommand: async () => ({ publicationReceipt: { call_id: `publication-${++gates}` }, postcondition: { satisfied: true },
        gateEnvelope: { policy_findings: outcomes[gates - 1] } }),
      checkpoint: async () => {},
      pause: async (id, evidence) => { pauses.push({ id, ...plain(evidence) }); if (!resumed) throw new Error('paused'); }
    };
    let result, error;
    try { result = await context.authorCandidate(w, { phase: 'body-TASK-A-001', prompt: () => 'author', retryScopes: new Set(['body', 'candidate_artifact']) }); }
    catch (caught) { error = caught.message; }
    return { context, pauses, gates, result, error };
  };
  const live = await run(recorded, false);
  assert.equal(live.error, 'paused', 'three attempts without progress pause the live run');
  assert.equal(live.gates, recorded.length);
  const resumed = await run([...recorded, ...after], true);
  assert.deepEqual(resumed.pauses, live.pauses, 'replay reaches the same pause with the same evidence');
  const evidence = live.pauses[0];
  assert.equal(evidence.best_open_findings, 3);
  assert.equal(evidence.subject_stages.length, 20, 'bounded like the other pause evidence');
  assert.equal(evidence.subject_stages_total, 32);
  assert.ok(evidence.subject_stages.every(([subject]) => subject.length <= 503));
  assert.equal(resumed.gates, recorded.length + after.length, 'after the resume one critic finding is fewer than three');
  assert.equal(resumed.result.pendingAuthor, undefined);
  // Credited advances in the history are bounded the same way.
  const progress = live.context.newProgress([]);
  live.context.recordAttempt(progress, 1, subjects.map(subject => judged(`${subject}-${'x'.repeat(600)}`, 'judge')));
  live.context.recordAttempt(progress, 2, subjects.map(subject => probe(`${subject}-${'x'.repeat(600)}`)));
  const advanced = progress.history.at(-1).advanced;
  assert.equal(advanced.length, 20);
  assert.ok(advanced.every(subject => subject.length <= 503));
}

function rewordingAJudgedFindingNeverCounts() {
  const texts = ['the command masks failures', 'the check masks empty runs', 'it greps its own output', 'it cannot fail'];
  const reworded = attempts(texts.map(text => [judged('AC-X-001', 'judge', text)]));
  assert.deepEqual(reworded.flags, [true, false, false, false]);
  assert.equal(reworded.reasons.at(-1), 'no_progress');
  // Two texts for one subject at one stage are one open finding, so merging
  // them is not fewer.
  const merged = attempts([[judged('AC-X-001', 'judge', 'a'), judged('AC-X-001', 'judge', 'b')],
    [judged('AC-X-001', 'judge', 'c')], [judged('AC-X-001', 'judge', 'd')], [judged('AC-X-001', 'judge', 'e')]]);
  assert.deepEqual(merged.flags, [true, false, false, false]);
  assert.equal(merged.reasons.at(-1), 'no_progress');
}

function firstSightingIsNoCredit() {
  // AC-X-002 first fails in an attempt without progress, so its advance is
  // not credited until an attempt with progress includes it (the bound).
  const { flags } = attempts([[judged('AC-X-001', 'judge')],
    [judged('AC-X-001', 'judge'), defect('AC-X-002', 'uncovered_requirement', 'covers')],
    [judged('AC-X-001', 'judge'), judged('AC-X-002', 'judge')]]);
  assert.deepEqual(flags, [true, false, false]);
  // A stage this script does not know ranks with the gate's validators.
  const unknown = attempts([[judged('AC-X-001', 'judge')], [judged('AC-X-001', 'judge')], [judged('AC-X-001', 'later-stage')]]);
  assert.deepEqual(unknown.flags, [true, false, false]);
}

function everyRandomSequencePauses() {
  let seed = 562;
  const random = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
  const subjects = ['AC-X-001', 'AC-X-002', 'AC-X-003', 'AC-X-004'];
  const kinds = [s => defect(s, 'uncovered_requirement', 'covers'), s => judged(s, 'judge'), s => probe(s),
    s => judged(s, 'passability'), s => judged(s, 'critic', 'c', 'REQ-1'), s => judged(s, 'critic', 'c', 'REQ-2')];
  for (let trial = 0; trial < 300; trial += 1) {
    const context = freshContext(), progress = context.newProgress([]);
    let count = 0;
    do {
      const findings = subjects.filter(() => random() < 0.6).map(s => kinds[Math.floor(random() * kinds.length)](s));
      context.recordAttempt(progress, ++count, findings.length > 0 ? findings : [judged(subjects[0], 'judge')]);
    } while (!context.stallReason(progress) && count < 2000);
    assert.equal(context.stallReason(progress), 'no_progress', `trial ${trial} did not pause in ${count} attempts`);
  }
}

const regressions = [
  ['R13 freezes never pause (#562)', r13FreezesNeverPause],
  ['single subject advancing through stages does not pause (#562)', singleSubjectAdvancingThroughStagesDoesNotPause],
  ['same stage repeated pauses (#562)', sameStageRepeatedPauses],
  ['traded early failure with a regression is no advance (#562)', tradedEarlyFailureWithARegressionIsNoAdvance],
  ['body critic findings shrinking (#562)', bodyCriticFindingsShrinking],
  ['set gate trade at the same total pauses (#562)', setGateTradeAtTheSameTotalPauses],
  ['replay rebuilds the ratchets (#562)', replayRebuildsTheRatchets],
  ['rewording a judged finding never counts (#562)', rewordingAJudgedFindingNeverCounts],
  ['first sighting is no credit (#562)', firstSightingIsNoCredit],
  ['every random sequence pauses (#562)', everyRandomSequencePauses]
];
(async () => {
  let failed = 0;
  for (const [name, test] of regressions) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (error) { failed += 1; console.error(`FAIL ${name}: ${error.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();

// Issue 567: a pre-judge syntax refusal has one validator identity beside
// a sibling's policy defect and after that sibling is repaired.
{
  const context = freshContext();
  const parse = defect('AC-X-001', 'shell_parse', 'check');
  const sibling = defect('AC-X-002', 'invalid_verifier', 'check');
  const before = [parse, sibling], after = [parse];
  assert.equal(context.openFindingKey(before[0]), context.openFindingKey(after[0]));
  assert(context.gateStageRank(parse) < context.gateStageRank(judged('AC-X-001', 'judge')),
    'a pre-judge parse refusal must rank below a judged finding');
  const regression = attempts([[judged('AC-X-001', 'judge')], [parse]]);
  assert.deepEqual(regression.flags, [true, false], 'regressing to syntax refusal is no progress');
}
