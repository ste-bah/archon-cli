// The failing-subject side of an attempt's progress (Issues 529, 561).
// Loaded after workflow_decompose_v1_progress.js as one script: every
// declaration here is hoisted into the same scope as `workflow`.

function failingSubjectKey(finding) {
  const defect = finding && finding.deterministic_defect;
  const subject = defect && typeof defect.subject === "string" && defect.subject.length > 0
    ? defect.subject : finding && typeof finding.subject === "string" && finding.subject.length > 0
      ? finding.subject : null;
  return subject === null ? findingKey(finding) : subject;
}

function isStrictSubset(set, best) {
  if (set.size >= best.size) return false;
  for (const value of set) if (!best.has(value)) return false;
  return true;
}

// Measures one attempt's failing set against the baseline and against the
// previous attempt (`progress.last`), and records both for the next attempt.
// `better`: the measure beat every earlier attempt, or the set is a strict
// subset of the baseline (#529); either way the set becomes the baseline
// (#561), so an attempt that sets a new best measure brings every subject
// that first fails there.
// `neutral`: not better, but a strict subset of the previous attempt's set at
// a measure no worse than that attempt's. A subject an earlier stage withheld
// can first fail after the best attempt; the attempts that then repair it
// neither add to nor reset the window. A smaller set behind a worse measure
// (an earlier stage hiding later ones) is not neutral.
// Bounded: a neutral attempt strictly shrinks the previous set, so between
// two attempts that add to the window there are at most |previous set|
// neutral ones; three that add pause the run; and real progress is finite
// (the measure improves finitely often, and each baseline admits at most its
// size of strict subsets). A trade, a repeated set or a growth adds one.
function failingSetStep(progress, measure, measureBetter) {
  const failing = measure.failingSubjects;
  const baseline = progress.bestFailing instanceof Set ? progress.bestFailing : null;
  const previous = progress.last && progress.last.failingSubjects instanceof Set ? progress.last : null;
  const better = measureBetter || (baseline !== null && isStrictSubset(failing, baseline));
  const neutral = !better && previous !== null && !isBetter(previous, measure)
    && isStrictSubset(failing, previous.failingSubjects);
  if (baseline === null || better) progress.bestFailing = failing;
  progress.last = measure;
  return { better, neutral };
}
