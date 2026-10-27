// Regressions for monotone failing-subject progress in acceptance rounds.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const files = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js',
  'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js',
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js'];
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

const regressions = [
  ['shrinking live sequence resets stall', shrinkingLiveSequenceResetsStall],
  ['reworded same set stalls', rewordedSameSetStalls],
  ['fixing one and adding one stalls', fixingOneAndAddingOneStalls],
  ['resume keeps best failing set', resumeKeepsBestFailingSet],
  ['missing subjects use finding keys and empty set passes', missingSubjectsUseFindingKeysAndEmptySetPasses],
  ['legacy progress initializes without subset credit', legacyProgressInitializesWithoutSubsetCredit]
];
let failed = 0;
for (const [name, test] of regressions) {
  try { test(); console.log(`PASS ${name}`); }
  catch (error) { failed += 1; console.error(`FAIL ${name}: ${error.message}`); }
}
process.exitCode = failed ? 1 : 0;
