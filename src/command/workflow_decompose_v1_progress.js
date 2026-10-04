// Progress-limited loops for the fixed decomposition script (Issue 261).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// A loop is limited by attempts that make NO progress, never by its total.
// An attempt is measured, lexicographically, as (tier, first failing
// deterministic stage, distinct deterministic defects at that stage), and it
// makes progress only when that measure beats every earlier attempt: a higher
// tier, a later stage (fixing a stage lets the next one run, and it may
// report more defects than the last did), or fewer defects at the same
// stage. Deterministic validators own the identities and stages; their
// diagnostics and submitted values cannot reset a window. The judge's
// free-text findings are not measured at all: they neither credit nor block.
// A stall PAUSES the run and resume opens a fresh window while preserving the
// best measure reconstructed by replay.

// Consecutive attempts without progress that end a loop: every kind counts,
// an outage, an incomplete reply and a judged repeat alike.
const STALL_ATTEMPTS = 3;
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
// Deterministic validation stages in pipeline order; mirrors the host's
// DefectStage (a test fails if they drift). An attempt with no deterministic
// defect has passed them all.
const DEFECT_STAGES = ["parse", "shape", "structure", "tools", "graph", "contracts", "freeze"];
const PASSED_STAGE = DEFECT_STAGES.length;
// The host's code-to-stage table (DefectStage::STAGED_CODES), for envelopes
// written before defects carried their stage; any other code is a contract
// check. A test fails if this drifts from the host's table.
const STAGE_CODES = {
  parse: ["invalid_json", "unreadable_task_file", "unparseable_task_file", "invalid_task_spec", "unreadable_task_spec", "unreadable_task_directory"],
  shape: ["invalid_candidate_shape", "unbound_candidate", "invalid_schema_version", "invalid_floor_serialization", "redacted_executable_value", "section_heading", "invalid_declared_status", "scope_declaration", "missing_runnable_test"],
  structure: ["invalid_task_id", "duplicate_task_id", "invalid_filename", "duplicate_filename", "empty_task_set", "empty_acceptance", "duplicate_acceptance_id", "duplicate_supplementary_id", "invalid_supplementary_id", "unknown_acceptance_id", "empty_criterion", "empty_judgment_field", "task_file_without_directory", "candidate_refused"],
  tools: ["tool_obligation"],
  graph: ["duplicate_dependency", "invalid_edge_declaration", "empty_consumed_path", "missing_dependency", "missing_blocked_task", "self_block", "contradictory_edge", "mutual_blocks", "dependency_cycle", "missing_producer", "missing_consumer_declaration", "missing_producer_deliverable", "missing_record_binding", "missing_producer_record_binding", "record_binding_mismatch", "kind_mismatch", "missing_data_obligation"],
  freeze: ["acceptance_digest_mismatch", "frozen_field_changed", "missing_frozen_task", "extra_task", "unreadable_acceptance_pin", "prd_identity_mismatch", "task_absent_from_skeleton", "partial_skeleton_freeze", "predecessor_findings", "invalid_acceptance_bundle", "invalid_skeleton_chain"],
};
const REFUSED_PREFIX = "candidate artifact was refused:";

function progressText(finding) {
  return typeof finding === "string" ? finding : String((finding && finding.text) || "");
}

function hostDefect(finding) {
  const defect = finding && finding.deterministic_defect;
  return defect && defect.provenance === "host_validator" && typeof defect.code === "string" ? defect : null;
}

// A finding the host decided, not the judge: a structured host identity, or
// one of the host's own refusal texts from an envelope that predates them.
function isDeterministic(finding) {
  if (hostDefect(finding)) return true;
  const text = progressText(finding);
  return text.includes(PACKAGING_REFUSAL) || text.startsWith(REFUSED_PREFIX);
}

// A refusal whose shape is wrong was parsed: it is refused, not packaging.
function findingTier(findings) {
  const tiers = findings.map((finding) => {
    const text = progressText(finding);
    const defect = hostDefect(finding);
    if (defect) {
      if (defect.code === "invalid_json") return PACKAGING_TIER;
      if (defect.code === "invalid_candidate_shape" || defect.code === "unbound_candidate") return REFUSED_TIER;
      return text.startsWith(REFUSED_PREFIX) ? REFUSED_TIER : JUDGED_TIER;
    }
    if (text.includes(PACKAGING_REFUSAL)) return PACKAGING_TIER;
    return text.startsWith(REFUSED_PREFIX) ? REFUSED_TIER : JUDGED_TIER;
  });
  // Mixed validator/judge reports reached the strongest reported tier.
  return tiers.reduce((tier, next) => Math.max(tier, next), PACKAGING_TIER);
}

// A host defect's stage, or its code's stage when the envelope predates
// stages. A host text refusal without an identity never claims a later stage
// than the first.
function stageOf(finding) {
  const defect = hostDefect(finding);
  if (!defect) return 0;
  const named = DEFECT_STAGES.indexOf(defect.stage);
  if (named >= 0) return named;
  const staged = Object.keys(STAGE_CODES).find((stage) => STAGE_CODES[stage].includes(defect.code));
  return DEFECT_STAGES.indexOf(staged || "contracts");
}

function attemptMeasure(findings) {
  const tier = findingTier(findings);
  const deterministic = findings.filter(isDeterministic);
  const stage = deterministic.reduce((first, finding) => Math.min(first, stageOf(finding)), PASSED_STAGE);
  const keys = deterministic.filter((finding) => stageOf(finding) === stage).map(findingKey);
  return { tier, stage, count: new Set(keys).size };
}

function isBetter(measure, best) {
  if (!best) return true;
  if (measure.tier !== best.tier) return measure.tier > best.tier;
  if (measure.stage !== best.stage) return measure.stage > best.stage;
  return measure.count < best.count;
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

function newProgress(_seed) {
  return {
    best: null,
    history: [],
    stalled: 0,
    stalledOperational: 0,
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

// Only a measure that beats every earlier attempt is progress. Trading,
// renaming, rewording or revisiting defects cannot reset the window, and
// neither can anything the judge says.
function recordAttempt(progress, call, findings, answered = true) {
  if (answered) progress.answered += 1;
  const measure = attemptMeasure(findings);
  const better = isBetter(measure, progress.best);
  if (better) progress.best = measure;
  return recordStep(progress, {
    call,
    kind: ["packaging", "refused", "judged"][measure.tier],
    stage: DEFECT_STAGES[measure.stage] || "passed",
    findings: measure.count,
    progress: better
  });
}

// Records an attempt the provider answered but nothing measured: an
// incomplete reply, or an acceptance round that ended on malformed replies.
// `advanced` is true only when the round completed a previously missing entry.
function recordAnswered(progress, call, kind, advanced = false, answered = true) {
  if (answered) progress.answered += 1;
  return recordStep(progress, { call, kind, findings: null, progress: Boolean(advanced) });
}

// Records an author call the provider never answered.
function recordOperational(progress, call, summary, advanced = false) {
  return recordStep(progress, { call, kind: "operational", findings: null, progress: Boolean(advanced), summary: boundText(summary) });
}

// Why the loop must stop now, or null while it may make another attempt.
function stallReason(progress) {
  if (progress.stalled >= STALL_ATTEMPTS) {
    return progress.stalledOperational >= progress.stalled ? "operational_no_progress" : "no_progress";
  }
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
    recovery: `The run is paused, not failed. Repair what the last findings name (the PRD, a gate, the provider), then resume the run: the resumed loop reuses every recorded attempt and gets a fresh window of ${STALL_ATTEMPTS} attempts without progress.`
  });
}

// Pauses a loop that `stallReason` stopped, then opens its fresh window.
async function pauseAuthorLoop(w, subject, progress, reason, lastFindings, extra) {
  await pauseLoop(w, subject, { ...loopEvidence(progress, reason, lastFindings), ...(extra || {}) });
  progress.stalled = 0;
  progress.stalledOperational = 0;
}
