// Progress-limited author loops for the fixed decomposition script (Issue 261).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// A loop is limited by attempts that make NO progress, never by its total.
// Fixed budgets (six acceptance attempts, ten per body, three operational)
// failed the whole decomposition on the attempt after the last one, however
// well the author was doing: a live body needed five attempts to converge,
// and one exhausted body ended a set of fifteen. Now an attempt that makes
// progress keeps its subject going; STALL_ATTEMPTS consecutive attempts that
// make none end the loop, and so does RUNAWAY_ATTEMPT_GUARD attempts in one
// window, which bounds only an author that reports progress forever. A loop
// that ends PAUSES the run with its evidence (`w.pause`); it never fails it.

// Consecutive attempts without progress that end a loop. Operational failures
// (the provider never answered) are counted apart, against the same number:
// an outage is not the author's, and an answer is the provider's progress.
const STALL_ATTEMPTS = 3;
// Author calls in one window after which even progressing attempts pause. It
// is not a work budget: progress is bounded by the findings an author can
// clear, so only an author reporting endless new findings ever reaches it.
const RUNAWAY_ATTEMPT_GUARD = 64;
// Bounds on what one pause event carries.
const PAUSE_EVIDENCE_FINDINGS = 20;
const PAUSE_EVIDENCE_TEXT = 500;

// How far an attempt's artifact got. A reply the host could not parse at all
// (packaging) is below one it parsed and refused without a judge, which is
// below one it judged: fewer findings at a lower tier is not an improvement.
const PACKAGING_TIER = 0;
const REFUSED_TIER = 1;
const JUDGED_TIER = 2;

function findingTier(texts) {
  if (texts.every((text) => text.includes(PACKAGING_REFUSAL))) return PACKAGING_TIER;
  if (texts.every((text) => text.startsWith("candidate artifact was refused:"))) return REFUSED_TIER;
  return JUDGED_TIER;
}

// What makes two findings the same defect. A standalone number is not: an
// author that keeps restating a line count is repeating one finding, not
// finding new ones. A number inside an identifier is (AC-X-001 and AC-X-002
// are two entries, src/v1.rs and src/v2.rs two files). A packaging refusal is
// one defect whatever the parser said.
function findingKey(text) {
  const value = String(text);
  return value.includes(PACKAGING_REFUSAL) ? PACKAGING_REFUSAL : value.replace(/(?<![\w.-])\d+(?:\.\d+)?/g, "#");
}

// One loop's progress record. `seed` is feedback the loop opened with (a set
// gate's findings): a later attempt repeating it has found nothing new.
function newProgress(seed) {
  return {
    best: null,
    seen: new Set((seed || []).map(findingKey)),
    history: [],
    stalled: 0,
    operational: 0,
    calls: 0,
    answered: 0,
    windowStart: 0
  };
}

// Records one answered attempt and returns whether it made progress.
// `findings` are the repairable findings of a judged or refused attempt, or
// null for an attempt nothing measured (an incomplete provider reply).
//
// Progress is either of:
// - a new best: a higher tier, or fewer findings at the best tier so far. The
//   loop already keeps the artifact with the fewest findings as its best
//   (attempts do not improve monotonically: live, 2, 1, 2, 1, 1);
// - an all-new finding set at the best tier: every earlier finding cleared,
//   and only defects no earlier attempt in the loop reported. That is an
//   author working through distinct defects (a mechanical refusal names one
//   at a time); trading a finding for one seen before is the oscillation the
//   attempt history exists to break, and is not progress.
function recordAttempt(progress, call, findings, kind) {
  progress.answered += 1;
  progress.operational = 0;
  let advanced = false;
  let tier = null;
  if (findings) {
    tier = findingTier(findings);
    const keys = findings.map(findingKey);
    const best = progress.best;
    const better = !best || tier > best.tier || (tier === best.tier && findings.length < best.count);
    const novel = Boolean(best) && tier === best.tier && keys.every((key) => !progress.seen.has(key));
    advanced = better || novel;
    if (better) progress.best = { tier, count: findings.length };
    for (const key of keys) progress.seen.add(key);
  }
  progress.stalled = advanced ? 0 : progress.stalled + 1;
  progress.history.push({
    call,
    kind: kind || ["packaging", "refused", "judged"][tier],
    findings: findings ? findings.length : null,
    progress: advanced
  });
  return advanced;
}

function recordOperational(progress, call, summary) {
  progress.operational += 1;
  progress.history.push({ call, kind: "operational", findings: null, progress: false, summary: boundText(summary) });
}

// Why the loop must stop now, or null while it may make another attempt.
function stallReason(progress) {
  if (progress.operational >= STALL_ATTEMPTS) return "operational_no_progress";
  if (progress.stalled >= STALL_ATTEMPTS) return "no_progress";
  if (progress.calls - progress.windowStart >= RUNAWAY_ATTEMPT_GUARD) return "runaway_guard";
  return null;
}

function boundText(text) {
  const value = String(text === undefined || text === null ? "" : text);
  return value.length > PAUSE_EVIDENCE_TEXT ? `${value.slice(0, PAUSE_EVIDENCE_TEXT)}...` : value;
}

function boundFindings(findings) {
  return (findings || []).slice(0, PAUSE_EVIDENCE_FINDINGS).map(boundText);
}

// The evidence an author loop's pause carries.
function loopEvidence(progress, reason, lastFindings) {
  return {
    reason,
    author_calls: progress.calls,
    answered_attempts: progress.answered,
    stall_window: STALL_ATTEMPTS,
    runaway_guard: RUNAWAY_ATTEMPT_GUARD,
    progress_history: progress.history.slice(-RUNAWAY_ATTEMPT_GUARD),
    progress_history_total: progress.history.length,
    last_findings: boundFindings(lastFindings),
    last_findings_total: (lastFindings || []).length
  };
}

// Pauses taken per subject. A subject the set gate re-opens keeps counting,
// so every pause of a run has its own id.
const PAUSES = new Map();

// Pauses the run for `subject` and returns only once a resumed run has passed
// this pause. The host takes a pause once per id: a resume replays the
// recorded attempts back to this call, the host answers that the pause was
// already taken, and the caller continues with a fresh window. Without that
// a resume would re-pause on the evidence it was resumed past, attempting
// nothing.
async function pauseLoop(w, subject, evidence) {
  const ordinal = (PAUSES.get(subject) || 0) + 1;
  PAUSES.set(subject, ordinal);
  await w.pause(`pause-${subject}-${ordinal}`, {
    subject,
    ordinal,
    ...evidence,
    recovery: `The run is paused, not failed. Repair what the last findings name (the PRD, a gate, the provider), then resume the run: the resumed loop reuses every recorded attempt and gets a fresh window of ${STALL_ATTEMPTS} attempts without progress.`
  });
}

// Pauses an author loop that `stallReason` stopped, then opens its fresh window.
async function pauseAuthorLoop(w, subject, progress, reason, lastFindings) {
  await pauseLoop(w, subject, loopEvidence(progress, reason, lastFindings));
  progress.stalled = 0;
  progress.operational = 0;
  progress.windowStart = progress.calls;
}
