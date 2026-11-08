// Regressions for monotone failing-subject progress in acceptance rounds.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const files = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js",
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js'];
const source = files.map(file => fs.readFileSync(`${scriptRoot}/${file}`, 'utf8')).join('\n');
const ctx = {};
vm.createContext(withAuthorContext(ctx));
vm.runInContext(source, ctx);
const finding = (subject, text = 'finding') => ({ subject, text, remediation_scope: 'candidate_artifact' });

function sequence(sets) {
  const progress = ctx.newProgress([]);
  const entries = sets.map((subjects, index) => {
    ctx.recordAttempt(progress, index + 1,
      subjects.map(subject => finding(subject, `round ${index + 1} ${subject}`)));
    return progress.history.at(-1);
  });
  return { progress, entries };
}

function shrinkingLiveSequenceResetsStall() {
  const { progress, entries } = sequence([
    ['AC-X-001', 'AC-X-002', 'AC-X-003', 'AC-X-004', 'AC-X-005', 'AC-X-006'],
    ['AC-X-001', 'AC-X-002', 'AC-X-003', 'AC-X-004', 'AC-X-005'],
    ['AC-X-001', 'AC-X-002', 'AC-X-003', 'AC-X-007', 'AC-X-008'],
    ['AC-X-001', 'AC-X-002']
  ]);
  assert.equal(entries[3].progress, true, 'the final failing set is a strict subset of the earlier best');
  assert.equal(progress.stalled, 0);
  assert.equal(ctx.stallReason(progress), null);
  assert.deepEqual(Array.from(progress.bestFailing), ['AC-X-001', 'AC-X-002']);
}

function rewordedSameSetStalls() {
  const { progress, entries } = sequence([
    ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-002']
  ]);
  assert.deepEqual(entries.map(entry => entry.progress), [true, false, false, false]);
  assert.equal(ctx.stallReason(progress), 'no_progress');
  assert.deepEqual(Array.from(progress.bestFailing), ['AC-X-001', 'AC-X-002']);
}

function fixingOneAndAddingOneStalls() {
  const { progress, entries } = sequence([
    ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-003'], ['AC-X-001', 'AC-X-004'], ['AC-X-001', 'AC-X-005']
  ]);
  assert.deepEqual(entries.map(entry => entry.progress), [true, false, false, false]);
  assert.equal(ctx.stallReason(progress), 'no_progress');
  assert.deepEqual(Array.from(progress.bestFailing), ['AC-X-001', 'AC-X-002']);
}

function resumeKeepsBestFailingSet() {
  const sets = [['AC-X-001', 'AC-X-002', 'AC-X-003'], ['AC-X-001', 'AC-X-002']];
  const { progress } = sequence(sets);
  const evidence = ctx.loopEvidence(progress, 'no_progress', []);
  assert.deepEqual(Array.from(evidence.best_failing), ['AC-X-001', 'AC-X-002']);
  const resumed = ctx.newProgress([]);
  for (const [index, subjects] of sets.entries()) {
    ctx.recordAttempt(resumed, index + 1, subjects.map(subject => finding(subject, `replayed ${subject}`)));
  }
  assert.deepEqual(Array.from(resumed.bestFailing), Array.from(evidence.best_failing));
}

function missingSubjectsUseFindingKeysAndEmptySetPasses() {
  const progress = ctx.newProgress([]);
  ctx.recordAttempt(progress, 1, [{ text: 'first no-subject finding' }, { text: 'second no-subject finding' }]);
  assert.equal(progress.bestFailing.size, 2);
  assert.equal(ctx.recordAttempt(progress, 2, []), true);
  assert.deepEqual(Array.from(progress.bestFailing), []);
}

function legacyProgressInitializesWithoutSubsetCredit() {
  const progress = ctx.newProgress([]);
  progress.best = { tier: 2, stage: 7, count: 0 };
  progress.bestFailing = undefined;
  progress.stalled = 2;
  ctx.recordAttempt(progress, 1, [finding('AC-X-001')]);
  assert.equal(progress.history.at(-1).progress, false);
  assert.equal(progress.stalled, 3);
  assert.deepEqual(Array.from(progress.bestFailing), ['AC-X-001']);
}

// #561: freezes of a live acceptance loop, as measured from its gate
// envelopes: [call, deterministic contract-stage count, failing subjects].
// The calls between them refused one entry each for its shape.
const DL = n => `SUP-REQ-DL-${String(n).padStart(3, '0')}`;
const AH = n => `SUP-REQ-AHDM-${String(n).padStart(3, '0')}`;
const LIVE_FREEZES = [
  [2, 49, ['AC-AHDM-001', 'AC-AHDM-002', 'AC-AHDM-003', 'AC-AHDM-004', 'AC-DL-001', 'AC-DL-002', 'AC-DL-003',
    'AC-DL-004', 'AC-DL-005', 'AC-DL-006', 'AC-DL-007', ...[2, 3, 4, 6, 7, 9, 10, 33].map(AH),
    ...[10, 11, 12, 13, 54, 56, 61, 62, 70, 71, 74, 85, 102, 122, 123, 124, 130, 131, 132, 133].map(DL)]],
  [6, 22, ['AC-AHDM-001', 'AC-AHDM-002', 'AC-AHDM-003', 'AC-AHDM-004', 'AC-DL-003', 'AC-DL-004', 'AC-DL-005',
    ...[1, 3, 4, 6, 7, 9, 21, 24, 30, 31, 32, 33].map(AH), 'SUP-REQ-BT-001', 'SUP-REQ-BT-004',
    ...[10, 12, 54, 72, 110, 111, 123, 124].map(DL)]],
  [9, 9, ['AC-AHDM-002', 'AC-DL-004', 'AC-DL-005', ...[20, 23, 33].map(AH), ...[73, 80, 81, 82, 110, 130].map(DL)]],
  [11, 1, ['AC-AHDM-002', AH(20), DL(80)]],
  [12, 0, ['AC-DL-003', 'AC-DL-004', ...[10, 54, 56, 61, 71, 72, 73, 82, 102, 110].map(DL)]],
  [13, 7, ['AC-DL-003', ...[2, 3, 4, 5, 73, 110].map(DL)]],
  [14, 1, ['AC-DL-003', DL(5), DL(110)]],
  [15, 1, [DL(110)]]
];
const LIVE_SHAPE_CALLS = [[1, 'AC-DL-001'], [3, 'AC-AHDM-001'], [4, 'AC-DL-003'], [5, AH(10)], [7, AH(24)],
  [8, DL(110)], [10, DL(81)]];
const plain = value => JSON.parse(JSON.stringify(value));
const det = (subject, index) => ({ subject, text: `contract ${index}`, remediation_scope: 'candidate_artifact',
  deterministic_defect: { provenance: 'host_validator', code: 'check_not_proven', subject, location: `slot-${index}`, stage: 'contracts' } });

// One freeze's findings: `count` distinct contract defects over its subjects,
// and a judged finding for each subject no defect names.
function freezeFindings(count, subjects) {
  const defects = Array.from({ length: count }, (_, index) => det(subjects[index % subjects.length], index));
  const named = new Set(defects.map(defect => defect.subject));
  return [...defects, ...subjects.filter(subject => !named.has(subject)).map(subject => finding(subject, `judged ${subject}`))];
}

function liveSteps(lastCall) {
  const steps = [...LIVE_FREEZES.map(([call, count, subjects]) => ({ call, findings: freezeFindings(count, subjects) })),
    ...LIVE_SHAPE_CALLS.map(([call, entryId]) => ({ call, refusal: { entryId, malformedClass: 'parse-error:other:location-unknown',
      findings: [{ text: 'not json', deterministic_defect: { provenance: 'host_validator', code: 'invalid_json', subject: entryId } }] } }))];
  return steps.filter(step => step.call <= lastCall).sort((a, b) => a.call - b.call);
}

// Feeds steps as the author loop does, pausing on a stall; `w.pause` answers
// as the host does on a resume that replays past an already taken pause.
async function drive(context, steps) {
  const progress = context.newProgress([]), pauses = [];
  const w = { pause: async (id, evidence) => { pauses.push({ id, ...evidence }); } };
  for (const step of steps) {
    if (step.refusal) context.recordRepairs(progress, step.call, [step.refusal]);
    else { context.recordAttempt(progress, step.call, step.findings); context.openRepairEpisode(progress); }
    const stall = context.stallReason(progress);
    if (stall) await context.pauseAuthorLoop(w, 'acceptance', progress, stall, []);
  }
  return { progress, pauses };
}

function freshContext() {
  const context = {};
  vm.createContext(withAuthorContext(context));
  vm.runInContext(source, context);
  return context;
}

async function liveFreezesNeverPauseWhileTheBestSetShrinks() {
  const { progress, pauses } = await drive(ctx, liveSteps(15));
  assert.deepEqual(plain(progress.history.map(entry => [entry.call, entry.kind, entry.stage, entry.findings])).filter(e => e[1] === 'judged'),
    LIVE_FREEZES.map(([call, count]) => [call, 'judged', call === 12 ? 'passed' : 'contracts', count]),
    'the fixture measures the live freezes');
  assert.deepEqual(pauses, [], 'no no_progress pause by call 15');
  assert.deepEqual(plain(progress.history.slice(-3).map(entry => entry.progress)), [false, false, true]);
  assert.deepEqual(Array.from(progress.bestFailing), [DL(110)]);
  assert.equal(ctx.stallReason(progress), null);
}

const judgedSet = subjects => subjects.map(subject => finding(subject, `judged ${subject}`));

function tradesAfterABestMeasureStillPause() {
  const progress = ctx.newProgress([]);
  // #562: AC-X-002 and AC-X-003 are judged from the first attempt on, so no
  // later attempt moves them to a later gate stage.
  const flags = [[...freezeFindings(3, ['AC-X-001']), ...judgedSet(['AC-X-002', 'AC-X-003'])], judgedSet(['AC-X-001', 'AC-X-004']),
    judgedSet(['AC-X-002', 'AC-X-003']), judgedSet(['AC-X-001', 'AC-X-003']), judgedSet(['AC-X-001', 'AC-X-002'])]
    .map((findings, index) => ctx.recordAttempt(progress, index + 1, findings));
  assert.deepEqual(flags, [true, true, false, false, false], 'a subset of an older set is no credit after a better measure');
  assert.equal(ctx.stallReason(progress), 'no_progress');
  assert.deepEqual(Array.from(progress.bestFailing), ['AC-X-001', 'AC-X-004']);
}

function growthWithoutABetterMeasurePauses() {
  const progress = ctx.newProgress([]);
  const flags = [['AC-X-001'], ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-002', 'AC-X-003'],
    ['AC-X-001', 'AC-X-002', 'AC-X-003', 'AC-X-004']].map((subjects, index) => ctx.recordAttempt(progress, index + 1, judgedSet(subjects)));
  assert.deepEqual(flags, [true, false, false, false]);
  assert.equal(ctx.stallReason(progress), 'no_progress');
  assert.deepEqual(Array.from(progress.bestFailing), ['AC-X-001']);
  // Any sequence at one measure pauses: each credit shrinks the baseline, and
  // only an attempt that adds to the window can be followed by neutral ones,
  // fewer than |universe| of them (each strictly shrinks a non-empty set).
  let seed = 561;
  const random = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
  const universe = ['AC-X-001', 'AC-X-002', 'AC-X-003', 'AC-X-004', 'AC-X-005', 'AC-X-006'];
  for (let trial = 0; trial < 200; trial += 1) {
    const run = ctx.newProgress([]);
    let attempts = 0, firstBaseline = 0;
    do {
      const subjects = universe.filter(() => random() < 0.6);
      ctx.recordAttempt(run, attempts + 1, judgedSet(subjects.length > 0 ? subjects : [universe[0]]));
      if (attempts === 0) firstBaseline = run.bestFailing.size;
      attempts += 1;
    } while (!ctx.stallReason(run) && attempts < 100);
    assert.equal(ctx.stallReason(run), 'no_progress', `trial ${trial} did not pause`);
    const perStrike = universe.length;
    assert.ok(attempts <= 1 + firstBaseline * (1 + 2 * perStrike) + 3 * perStrike, `trial ${trial} took ${attempts} attempts`);
  }
}

async function replayRebuildsTheBaseline() {
  const best = LIVE_FREEZES.find(([call]) => call === 12)[2];
  const trades = [1, 2, 3].map(index => best.map((subject, at) => at === index ? `AC-X-90${index}` : subject));
  const steps = [...liveSteps(12), ...trades.map((subjects, index) => ({ call: 13 + index, findings: judgedSet(subjects) })),
    { call: 16, findings: judgedSet([DL(110)]) }];
  const live = await drive(freshContext(), steps);
  assert.equal(live.pauses.length, 1, 'three trades of the best set pause once');
  assert.deepEqual(plain(live.pauses[0].best_failing), best, 'the pause carries the best attempt as its baseline');
  const replayed = await drive(freshContext(), steps);
  assert.deepEqual(plain(replayed.progress.best), plain(live.progress.best));
  assert.deepEqual(Array.from(replayed.progress.bestFailing), Array.from(live.progress.bestFailing));
  assert.deepEqual(plain(replayed.progress.history), plain(live.progress.history));
  assert.deepEqual(plain(replayed.pauses), plain(live.pauses));
  assert.equal(live.progress.history.at(-1).progress, true, 'after the resume a subset of the baseline is progress');
}

// Records judged sets at one measure (the maximum): each attempt's
// progress flag, neutral flag, window and stall reason.
function windowOf(context, sets) {
  const progress = context.newProgress([]);
  const steps = sets.map((subjects, index) => {
    const result = context.recordAttempt(progress, index + 1, judgedSet(subjects));
    const entry = progress.history.at(-1);
    return { progress: result, neutral: Boolean(entry.neutral), stalled: progress.stalled, reason: context.stallReason(progress) };
  });
  return { progress, steps };
}

function tradeCycleStillPauses() {
  const cycle = [['S-1', 'S-4'], ['S-1'], ['S-1', 'S-5'], ['S-1'], ['S-1', 'S-5'], ['S-1'], ['S-1', 'S-5']];
  const { progress, steps } = windowOf(ctx, cycle);
  assert.deepEqual(steps.map(step => step.progress), [true, true, false, false, false, false, false]);
  assert.deepEqual(steps.map(step => step.neutral), [false, false, false, true, false, true, false]);
  assert.deepEqual(steps.map(step => step.stalled), [0, 0, 1, 1, 2, 2, 3], 'a shrink back is neutral, each rise adds one');
  assert.deepEqual(steps.map(step => step.reason), [null, null, null, null, null, null, 'no_progress']);
  const replayed = windowOf(freshContext(), cycle);
  assert.deepEqual(plain(replayed.progress.history), plain(progress.history), 'replay rebuilds the same window');
  assert.deepEqual(Array.from(replayed.progress.last.failingSubjects), Array.from(progress.last.failingSubjects));
}

function lateRevealAtTheMaximumMeasure() {
  const repeated = windowOf(ctx, [['AC-X-001'], [DL(110), DL(5)], [DL(110)], [DL(110)], [DL(110)]]);
  assert.deepEqual(repeated.steps.map(step => step.stalled), [0, 1, 1, 2, 3],
    'only the three attempts that do not shrink the previous set fill the window');
  assert.equal(repeated.steps.at(-1).reason, 'no_progress');
  const revealed = windowOf(ctx, [['AC-X-001'], ['AC-X-101', 'AC-X-102', 'AC-X-103'], ['AC-X-101', 'AC-X-102'], ['AC-X-101'], []]);
  assert.deepEqual(revealed.steps.map(step => step.stalled), [0, 1, 1, 1, 0]);
  assert.deepEqual(revealed.steps.map(step => step.reason), [null, null, null, null, null], 'a reveal that shrinks never pauses');
  assert.deepEqual(revealed.steps.map(step => step.progress), [true, false, false, false, true]);
  // A smaller set behind a worse measure (an earlier stage hiding later ones) is not neutral.
  const progress = ctx.newProgress([]);
  ctx.recordAttempt(progress, 1, judgedSet(['AC-X-001']));
  ctx.recordAttempt(progress, 2, judgedSet(['AC-X-101', 'AC-X-102']));
  ctx.recordAttempt(progress, 3, freezeFindings(1, ['AC-X-101']));
  assert.equal(progress.history.at(-1).neutral, undefined);
  assert.equal(progress.stalled, 2);
}

function sameSetRepeatedPauses() {
  const { steps } = windowOf(ctx, [['AC-X-001'], ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-002'], ['AC-X-001', 'AC-X-002']]);
  assert.deepEqual(steps.map(step => step.neutral), [false, false, false, false]);
  assert.deepEqual(steps.map(step => step.stalled), [0, 1, 2, 3]);
  assert.equal(steps.at(-1).reason, 'no_progress');
}

const regressions = [
  ['shrinking live sequence resets stall', shrinkingLiveSequenceResetsStall],
  ['reworded same set stalls', rewordedSameSetStalls],
  ['fixing one and adding one stalls', fixingOneAndAddingOneStalls],
  ['resume keeps best failing set', resumeKeepsBestFailingSet],
  ['missing subjects use finding keys and empty set passes', missingSubjectsUseFindingKeysAndEmptySetPasses],
  ['legacy progress initializes without subset credit', legacyProgressInitializesWithoutSubsetCredit],
  ['live freezes never pause while the best set shrinks (#561)', liveFreezesNeverPauseWhileTheBestSetShrinks],
  ['trades after a best measure still pause (#561)', tradesAfterABestMeasureStillPause],
  ['growth without a better measure pauses (#561)', growthWithoutABetterMeasurePauses],
  ['replay rebuilds the baseline (#561)', replayRebuildsTheBaseline],
  ['trade cycle still pauses (#561)', tradeCycleStillPauses],
  ['late reveal at the maximum measure (#561)', lateRevealAtTheMaximumMeasure],
  ['same set repeated pauses (#561)', sameSetRepeatedPauses]
];
(async () => {
  let failed = 0;
  for (const [name, test] of regressions) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (error) { failed += 1; console.error(`FAIL ${name}: ${error.message}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
