// Progress-limited loops for the fixed decomposition script (Issue 261).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// A loop is limited by attempts that make NO progress, never by its total.
// Fixed budgets (six acceptance attempts, ten per body, three operational)
// failed the whole decomposition on the attempt after the last one, however
// well the author was doing: a live body needed five attempts to converge,
// and one exhausted body ended a set of fifteen. Every loop -- each subject's
// author loop and the set-gate rounds -- now keeps one record: an attempt
// that makes progress keeps it going, and STALL_ATTEMPTS consecutive attempts
// that make none end it. A loop that ends PAUSES the run with its evidence
// (`w.pause`); it never fails it.

// Consecutive attempts without progress that end a loop: every kind counts,
// an outage, an incomplete reply and a judged repeat alike.
const STALL_ATTEMPTS = 3;
// Consecutive attempts whose only progress is novelty -- findings no earlier
// attempt reported, with no new best -- after which the loop pauses. A loop
// whose best keeps improving never reaches it, whatever its length: each new
// best restarts the count. It stops only an author that keeps trading its
// findings for new ones without ever converging.
const RUNAWAY_NOVELTY_GUARD = 64;
// Bounds on what one pause event carries.
const PAUSE_EVIDENCE_FINDINGS = 20;
const PAUSE_EVIDENCE_TEXT = 500;

// How far an attempt's artifact got. A reply the host could not parse at all
// (packaging) is below one it parsed and refused without a judge, which is
// below one it judged: fewer findings at a lower tier is not an improvement.
const PACKAGING_TIER = 0;
const REFUSED_TIER = 1;
const JUDGED_TIER = 2;

function progressText(finding) {
  return typeof finding === "string" ? finding : String((finding && finding.text) || "");
}

function findingTier(findings) {
  const texts = findings.map(progressText);
  if (texts.every((text) => text.includes(PACKAGING_REFUSAL))) return PACKAGING_TIER;
  if (texts.every((text) => text.startsWith("candidate artifact was refused:"))) return REFUSED_TIER;
  return JUDGED_TIER;
}

// What makes two findings the same defect: the subject and path the gate
// names it by, and its words. Case, spacing and punctuation are not words, so
// restating a finding is not finding a new one. Numbers are: argument 1 and
// argument 4, or AC-X-001 and AC-X-002, are different defects. A packaging
// refusal is one defect whatever the parser said.
function findingKey(finding) {
  const text = progressText(finding);
  if (text.includes(PACKAGING_REFUSAL)) return PACKAGING_REFUSAL;
  const words = text.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
  const subject = typeof finding === "object" && finding && typeof finding.subject === "string" ? finding.subject : "";
  const path = typeof finding === "object" && finding && typeof finding.source_path === "string" ? finding.source_path : "";
  return `${subject}\u0000${path}\u0000${words}`;
}

// One loop's progress record. `seed` is feedback the loop opened with (a set
// gate's findings): a later attempt repeating it has found nothing new.
function newProgress(seed) {
  return {
    best: null,
    seen: new Set((seed || []).map(findingKey)),
    history: [],
    stalled: 0,
    stalledOperational: 0,
    novelty: 0,
    calls: 0,
    answered: 0
  };
}

function recordStep(progress, entry) {
  if (entry.progress) {
    progress.stalled = 0;
    progress.stalledOperational = 0;
  } else {
    progress.stalled += 1;
    if (entry.kind === "operational") progress.stalledOperational += 1;
  }
  progress.history.push(entry);
  return entry.progress;
}

// Records one judged or refused attempt and returns whether it made progress.
// `findings` are its repairable findings (gate finding objects, or texts).
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
function recordAttempt(progress, call, findings) {
  progress.answered += 1;
  const tier = findingTier(findings);
  const keys = findings.map(findingKey);
  const best = progress.best;
  const better = !best || tier > best.tier || (tier === best.tier && findings.length < best.count);
  const novel = !better && tier === best.tier && keys.every((key) => !progress.seen.has(key));
  if (better) {
    progress.best = { tier, count: findings.length };
    progress.novelty = 0;
  } else if (novel) {
    progress.novelty += 1;
  }
  for (const key of keys) progress.seen.add(key);
  return recordStep(progress, {
    call,
    kind: ["packaging", "refused", "judged"][tier],
    findings: findings.length,
    progress: better || novel
  });
}

// Records an attempt the provider answered but nothing measured: an
// incomplete reply, or an acceptance round that ended on malformed replies.
// `advanced` is true only when the round completed work (an entry).
function recordAnswered(progress, call, kind, advanced) {
  progress.answered += 1;
  return recordStep(progress, { call, kind, findings: null, progress: Boolean(advanced) });
}

// Records an author call the provider never answered.
function recordOperational(progress, call, summary) {
  return recordStep(progress, { call, kind: "operational", findings: null, progress: false, summary: boundText(summary) });
}

// Why the loop must stop now, or null while it may make another attempt.
function stallReason(progress) {
  if (progress.stalled >= STALL_ATTEMPTS) {
    return progress.stalledOperational >= progress.stalled ? "operational_no_progress" : "no_progress";
  }
  if (progress.novelty >= RUNAWAY_NOVELTY_GUARD) return "runaway_guard";
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
    runaway_guard: RUNAWAY_NOVELTY_GUARD,
    progress_history: progress.history.slice(-RUNAWAY_NOVELTY_GUARD),
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
// recorded attempts back to this call verbatim, the host answers that the pause was
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

// Pauses a loop that `stallReason` stopped, then opens its fresh window.
async function pauseAuthorLoop(w, subject, progress, reason, lastFindings, extra) {
  await pauseLoop(w, subject, { ...loopEvidence(progress, reason, lastFindings), ...(extra || {}) });
  progress.stalled = 0;
  progress.stalledOperational = 0;
  progress.novelty = 0;
}
