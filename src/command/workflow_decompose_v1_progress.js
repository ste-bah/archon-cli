// Progress-limited loops for the fixed decomposition script (Issue 261).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// A loop is limited by attempts that make NO progress, never by its total.
// An attempt is measured as (tier, first failing deterministic stage,
// distinct deterministic defects across ALL stages), and it makes progress
// only when that measure beats every earlier attempt: a higher tier, a later
// first failing stage (fixing a stage lets the next one run, and it may
// report more defects than the last did), or, at the same first failing
// stage, fewer defects in total -- a repair at a later stage while an earlier
// defect remains is a real, independent repair. Deterministic validators own the identities and stages; their
// diagnostics and submitted values cannot reset a window. The judge's
// free-text findings are not measured at all: they neither credit nor block.
// A stall PAUSES the run and resume opens a fresh window while preserving the
// best measure reconstructed by replay.
//
// Issue 357: an entry the author step refuses for its shape is measured in its
// own repair frontier, not against the candidate best: a judged candidate is a
// higher tier than any entry shape refusal, so every shape repair after a
// refutation used to measure as a regression. Each candidate measure opens a
// repair episode (per-entry bests, and the window as that measure left it). An
// entry's strictly better shape measure cancels only the no-progress attempts
// of its episode; attempts before the episode still count, so a candidate
// that keeps coming back refuted still pauses.

// Independent consecutive windows: author attempts exclude operational
// failures, which the author cannot repair.
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
  return { tier, stage, count: new Set(deterministic.map(findingKey)).size };
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
    repair: newRepairEpisode(0),
    history: [],
    stalled: 0,
    stalledOperational: 0,
    calls: 0,
    answered: 0
  };
}

function newRepairEpisode(stalled) {
  return { bests: new Map(), malformedClasses: new Map(), stalled };
}

// Opens the repair episode that follows a measured candidate.
function openRepairEpisode(progress) {
  progress.repair = newRepairEpisode(progress.stalled);
}

function recordStep(progress, entry, gateSucceeded = false) {
  const operational = entry.kind === "operational" || entry.outage === true;
  if (entry.progress) {
    progress.stalled = 0;
    // Real progress empties the window, so no episode may restore it.
    progress.repair.stalled = 0;
  } else if (!operational) {
    progress.stalled += 1;
  }
  if (operational) progress.stalledOperational += 1;
  else if (gateSucceeded) progress.stalledOperational = 0;
  progress.history.push(entry);
  return entry.progress;
}

// What one author round reports to the window: the entries that passed in
// it and whether any of its calls failed in transport. A loop without an
// entry author has no rounds (NO_ROUND).
const NO_ROUND = Object.freeze({ passed: Object.freeze([]), outage: false });

function authorRoundReport(policy, state) {
  if (!policy.author) return NO_ROUND;
  if (!Array.isArray(state.roundPassed) || typeof state.roundOutage !== "boolean") {
    throw new Error("an author round reported no passed entries or outage");
  }
  return { passed: state.roundPassed, outage: state.roundOutage };
}

// Only a measure that beats every earlier attempt is progress. Trading,
// renaming, rewording or revisiting defects cannot reset the window, and
// neither can anything the judge says. The entries that passed in the clean
// round that made this candidate are credited first (creditPassed): that
// restores the window to the episode floor, and the judged measure is then
// recorded as usual, so a candidate the judge keeps refusing still raises
// every next floor by one and the loop stays bounded.
function recordAttempt(progress, call, findings, answered = true, round = NO_ROUND) {
  if (answered) progress.answered += 1;
  const credited = creditPassed(progress.repair, round.passed);
  if (credited.length > 0) restoreFloor(progress);
  const measure = attemptMeasure(findings);
  const better = isBetter(measure, progress.best);
  if (better) progress.best = measure;
  const entry = {
    call,
    kind: ["packaging", "refused", "judged"][measure.tier],
    stage: DEFECT_STAGES[measure.stage] || "passed",
    findings: measure.count,
    progress: better,
    ...(credited.length > 0 ? { entries: credited } : {})
  };
  if (round.outage) entry.outage = true;
  return recordStep(progress, entry, true);
}

// The measure of an entry that passed the author step: better than any refusal.
const PASSED_MEASURE = { tier: JUDGED_TIER, stage: PASSED_STAGE, count: 0 };

// Credits each entry that passed this round and was measured (refused) in
// the current repair episode: its first pass beats its best. A pass is the
// maximum, so each entry is credited at most once per episode. An entry with
// no measure in the episode (a rewrite of one the judge sent back) is only
// novelty until the judge accepts it (Issue 261). Returns the credited entries.
function creditPassed(episode, passed) {
  const credited = [];
  if (!Array.isArray(passed)) throw new Error("an author round reported no passed entries");
  for (const id of passed) {
    const best = episode.bests.get(id);
    if (!best || !isBetter(PASSED_MEASURE, best)) continue;
    episode.bests.set(id, PASSED_MEASURE);
    credited.push({ subject: id, findings: 0, progress: true, worse: false });
  }
  return credited;
}

// Closes one author round. `advanced` (a previously missing entry was
// completed) empties the window. An entry that beat its own best restores the
// window to where the repair episode opened, never below it, so attempts
// before the episode still count. Otherwise the round is one more attempt
// without progress.
function recordRound(progress, entry, better, advanced, outage, gateSucceeded = false) {
  entry.progress = better || Boolean(advanced);
  if (outage) entry.outage = true;
  if (advanced || !better) return recordStep(progress, entry, gateSucceeded);
  restoreFloor(progress);
  if (entry.kind === "operational" || entry.outage === true) progress.stalledOperational += 1;
  else if (gateSucceeded) progress.stalledOperational = 0;
  progress.history.push(entry);
  return true;
}

function restoreFloor(progress) {
  progress.stalled = Math.min(progress.stalled, progress.repair.stalled);
}

// Records one author round that refused entries for their shape. Each refused
// entry is measured against its own best in the current repair episode (a
// first measure beats nothing), and each entry that passed this round is
// credited (creditPassed). The round is progress when at least one entry beat
// its own best. An entry worse than its best does not cancel another entry's
// progress: each best only improves, over a finite measure (tier, stage, a
// defect count >= 0; a pass is terminal), so an episode holds finitely many
// improvements and the loop stays bounded, while a stuck or worse entry keeps
// reading its own refusal. `worse` is kept per entry as evidence.
function recordRepairs(progress, call, refusals, round = NO_ROUND, answered = true, advanced = false) {
  if (answered) progress.answered += 1;
  const episode = progress.repair;
  let better = false;
  const entries = [], measures = [];
  let malformedCount = 0, validatorCount = 0;
  for (const { entryId, findings, malformedClass } of refusals) {
    if (typeof entryId !== "string" || entryId.length === 0) throw new Error("an entry shape refusal names no entry");
    if (typeof malformedClass === "string") {
      let classes = episode.malformedClasses.get(entryId);
      if (!classes) episode.malformedClasses.set(entryId, classes = new Set());
      const improved = !classes.has(malformedClass);
      classes.add(malformedClass);
      better = better || improved;
      malformedCount += 1;
      entries.push({ subject: entryId, malformed_class: malformedClass,
        findings: findings.filter(isDeterministic).length, progress: improved, worse: false });
      continue;
    }
    const measure = attemptMeasure(findings);
    const best = episode.bests.get(entryId);
    const improved = isBetter(measure, best);
    const regressed = Boolean(best) && isBetter(best, measure);
    if (improved) episode.bests.set(entryId, measure);
    better = better || improved;
    measures.push(measure);
    validatorCount += 1;
    entries.push({ subject: entryId, findings: measure.count, progress: improved, worse: regressed });
  }
  const credited = creditPassed(episode, round.passed);
  entries.push(...credited);
  const tier = measures.length ? Math.min(...measures.map(measure => measure.tier)) : 0;
  const stage = measures.length ? Math.min(...measures.map(measure => measure.stage)) : 0;
  return recordRound(progress, {
    call, kind: malformedCount > 0 && validatorCount === 0 ? "malformed" : ["packaging", "refused", "judged"][tier], subject: refusals.map(refusal => refusal.entryId).join(", "),
    stage: malformedCount > 0 && validatorCount === 0 ? "parse" : DEFECT_STAGES[stage] || "passed",
    findings: measures.reduce((sum, measure) => sum + measure.count, 0)
      + entries.filter(entry => entry.malformed_class).reduce((sum, entry) => sum + entry.findings, 0),
    progress: false, entries
  }, better || credited.length > 0, advanced, round.outage);
}

// Malformed replies have stable refusal classes (wrong id, missing end_turn,
// parse-error kind). A new class is progress once per entry and episode;
// repeating it is not. Offsets and diagnostic wording are never keys.
// A round whose failures measured nothing (an unparseable reply, a call that
// failed in transport) still credits the entries that passed in it.
function creditedRound(progress, entry, round, advanced) {
  const credited = creditPassed(progress.repair, round.passed);
  if (credited.length > 0) entry.entries = credited;
  return recordRound(progress, entry, credited.length > 0, advanced, round.outage && entry.kind !== "operational");
}

// Records an attempt the provider answered but nothing measured: an
// incomplete reply, or an acceptance round that ended on malformed replies.
// `advanced` is true only when the round completed a previously missing
// entry; `round` is the author round's report (authorRoundReport).
function recordAnswered(progress, call, kind, advanced = false, answered = true, round = NO_ROUND) {
  if (answered) progress.answered += 1;
  return creditedRound(progress, { call, kind, findings: null, progress: false }, round, advanced);
}

// Records an author call the provider never answered, or a gate that never
// judged the candidate; `round` is the author round that made the attempt. A
// pass in it, or a previously missing entry it added (`advanced`), is
// progress whatever the gate does next: an outage measures nothing, a pass is
// credited once per episode and an entry is added once, so this stays bounded.
function recordOperational(progress, call, summary, advanced = false, round = NO_ROUND, component = "provider") {
  return creditedRound(progress, { call, kind: "operational", findings: null, progress: false,
    summary: `${component}: ${boundText(summary)}` }, round, advanced);
}

// Why the loop must stop now, or null while it may make another attempt.
function stallReason(progress) {
  if (progress.stalledOperational >= STALL_ATTEMPTS) return "operational_no_progress";
  if (progress.stalled >= STALL_ATTEMPTS) return "no_progress";
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
  const components = progressOperationalComponents(evidence.progress_history);
  const recovery = evidence.reason === "operational_no_progress"
    ? `The run is paused, not failed. Restore the host ${components.join(" and ") || "gate/provider"} component, then resume the run: the resumed loop reuses every recorded attempt and gets a fresh window of ${STALL_ATTEMPTS} attempts without progress.`
    : `The run is paused, not failed. Repair what the last findings name (the PRD, a gate, the provider), then resume the run: the resumed loop reuses every recorded attempt and gets a fresh window of ${STALL_ATTEMPTS} attempts without progress.`;
  await w.pause(`pause-${subject}-${ordinal}`, {
    subject,
    ordinal,
    ...evidence,
    recovery
  });
}

function progressOperationalComponents(history) {
  if (!Array.isArray(history)) return [];
  const components = [];
  for (let index = history.length - 1; index >= 0; index -= 1) {
    const entry = history[index];
    if (entry.kind !== "operational" && entry.outage !== true) break;
    if (entry.outage === true) components.push("provider");
    if (entry.kind === "operational") {
      const label = typeof entry.summary === "string" ? entry.summary.split(":", 1)[0] : "gate";
      components.push(label === "gate" ? "gate" : "provider");
    }
  }
  return [...new Set(components)];
}

// Pauses a loop that `stallReason` stopped, then opens its fresh window.
async function pauseAuthorLoop(w, subject, progress, reason, lastFindings, extra) {
  await pauseLoop(w, subject, { ...loopEvidence(progress, reason, lastFindings), ...(extra || {}) });
  progress.stalled = 0;
  progress.stalledOperational = 0;
  // The fresh window is also the episode's floor; its per-entry bests stay.
  progress.repair.stalled = 0;
}
