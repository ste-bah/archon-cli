// Progress-limited loops for the fixed decomposition script (Issue 261).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// A loop is limited by attempts that make NO progress, never by its total.
// Progress is a higher tier or a strictly smaller distinct defect count than
// any earlier attempt at that tier. Deterministic validators own structured
// identities; their diagnostics, submitted values and finding-set novelty
// cannot reset a window. A stall PAUSES the run and resume opens a fresh
// window while preserving the best measure reconstructed by replay.

// Consecutive attempts without progress that end a loop: every kind counts,
// an outage, an incomplete reply and a judged repeat alike.
const STALL_ATTEMPTS = 3;
// Consecutive attempts without real progress that end a loop. Every other
// attempt counts, judged novelty included; real progress restarts it.
const NO_NEW_BEST_ATTEMPTS = 64;
// Bounds on what one pause event carries.
const PAUSE_EVIDENCE_FINDINGS = 20;
const PAUSE_EVIDENCE_HISTORY = 64;
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
  const tiers = findings.map((finding) => {
    const text = progressText(finding);
    const defect = finding && finding.deterministic_defect;
    if (defect && defect.provenance === "host_validator") {
      if (defect.code === "invalid_json" || defect.code === "invalid_candidate_shape") return PACKAGING_TIER;
      if (defect.code === "unbound_candidate") return REFUSED_TIER;
      return text.startsWith("candidate artifact was refused:") ? REFUSED_TIER : JUDGED_TIER;
    }
    if (text.includes(PACKAGING_REFUSAL)) return PACKAGING_TIER;
    return text.startsWith("candidate artifact was refused:") ? REFUSED_TIER : JUDGED_TIER;
  });
  // Mixed validator/judge reports reached the strongest reported tier.
  return tiers.reduce((tier, next) => Math.max(tier, next), PACKAGING_TIER);
}

// Host-produced deterministic identities never use diagnostic prose or rejected
// values. Older envelopes and judged findings retain a conservative text key:
// rewording can change identity, but cannot count as progress at the same count.
function findingKey(finding) {
  const defect = finding && finding.deterministic_defect;
  if (defect && defect.provenance === "host_validator" && typeof defect.code === "string") {
    return JSON.stringify([defect.code, defect.subject || "", defect.location || ""]);
  }
  const text = progressText(finding);
  if (text.includes(PACKAGING_REFUSAL)) return PACKAGING_REFUSAL;
  const words = text.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
  const subject = typeof finding === "object" && finding && typeof finding.subject === "string" ? finding.subject : "";
  const path = typeof finding === "object" && finding && typeof finding.source_path === "string" ? finding.source_path : "";
  return JSON.stringify([subject, path, words]);
}

function findingCount(findings) {
  return new Set(findings.map(findingKey)).size;
}

function newProgress(_seed) {
  return {
    best: null,
    history: [],
    stalled: 0,
    stalledOperational: 0,
    sinceBest: 0,
    calls: 0,
    answered: 0
  };
}

// `entry.progress` keeps the short window open; `best` is real progress.
function recordStep(progress, entry, best) {
  progress.sinceBest = best ? 0 : progress.sinceBest + 1;
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

// Only a higher tier or a new minimum count at the best tier is progress.
// Trading, renaming, rewording or revisiting defects cannot reset the counter.
function recordAttempt(progress, call, findings, answered = true) {
  if (answered) progress.answered += 1;
  const tier = findingTier(findings);
  const count = findingCount(findings);
  const best = progress.best;
  const better = !best || tier > best.tier || (tier === best.tier && count < best.count);
  if (better) progress.best = { tier, count };
  return recordStep(progress, {
    call,
    kind: ["packaging", "refused", "judged"][tier],
    findings: count,
    progress: better
  }, better);
}

// What a round without a candidate retained. An entry no earlier round
// completed is a new best: the set of entries only grows, to the criteria.
// A rewrite of a completed entry clears no outstanding entry.
const ADVANCE_NONE = { progress: false, best: false };
const ADVANCE_BEST = { progress: true, best: true };

// Records an attempt the provider answered but nothing measured: an
// incomplete reply, or an acceptance round that ended on malformed replies.
function recordAnswered(progress, call, kind, advance = ADVANCE_NONE, answered = true) {
  if (answered) progress.answered += 1;
  return recordStep(progress, { call, kind, findings: null, progress: advance.progress }, advance.best);
}

// Records an author call the provider never answered.
function recordOperational(progress, call, summary, advance = ADVANCE_NONE) {
  return recordStep(progress, { call, kind: "operational", findings: null, progress: advance.progress, summary: boundText(summary) }, advance.best);
}

// Why the loop must stop now, or null while it may make another attempt.
function stallReason(progress) {
  if (progress.stalled >= STALL_ATTEMPTS) {
    return progress.stalledOperational >= progress.stalled ? "operational_no_progress" : "no_progress";
  }
  if (progress.sinceBest >= NO_NEW_BEST_ATTEMPTS) return "no_new_best";
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
    no_new_best_window: NO_NEW_BEST_ATTEMPTS,
    attempts_since_best: progress.sinceBest,
    progress_history: progress.history.slice(-PAUSE_EVIDENCE_HISTORY),
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
    recovery: `The run is paused, not failed. Repair what the last findings name (the PRD, a gate, the provider), then resume the run: the resumed loop reuses every recorded attempt and gets a fresh window of ${STALL_ATTEMPTS} attempts without progress and ${NO_NEW_BEST_ATTEMPTS} without a new best.`
  });
}

// Pauses a loop that `stallReason` stopped, then opens its fresh window.
async function pauseAuthorLoop(w, subject, progress, reason, lastFindings, extra) {
  await pauseLoop(w, subject, { ...loopEvidence(progress, reason, lastFindings), ...(extra || {}) });
  progress.stalled = 0;
  progress.stalledOperational = 0;
  progress.sinceBest = 0;
}
