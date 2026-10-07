// Phase seeds for the fixed decomposition script (Issue 360).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// After a script, catalog or template upgrade, replaying the old calls keeps
// almost nothing: each acceptance author prompt carries the other entries
// and earlier findings, so one changed decision changes every later input.
// A resume after an upgrade therefore starts each author loop from its
// latest durable candidate. The host reads it from the run's own records
// (workflow_decompose_seed.rs) and passes it as `args.phaseSeed`; this file
// turns it into the loop's state with this script's own rules.
//
// Nothing carried is accepted here. An acceptance entry is carried only when
// this build's entry validator passes it and the last gate did not refute it
// (or it was re-authored since); every other entry is authored again, and the
// assembled candidate goes to the acceptance gate. A whole artifact the last
// gate judged with findings is authored again with them; one never judged, or
// judged clean, is submitted as it is. A submission whose bytes a recorded
// gate already judged keeps that recorded verdict: the host replays it, so a
// completed skeleton or body is not judged again by this build's gate
// (Issue 361 tracks re-judging it under a changed gate).

// Subjects whose seed this run applied: a subject the set gate re-opens later
// continues from its live state, not from the seed again.
const SEEDED_SUBJECTS = new Set();

function phaseSeed() {
  const seed = args.phaseSeed;
  if (seed === undefined || seed === null) return null;
  if (typeof seed !== "object" || Array.isArray(seed) || !seed.subjects || typeof seed.subjects !== "object") {
    throw new Error("fixed decomposition phaseSeed argument is malformed");
  }
  return seed;
}

// Seeded calls and pauses continue after every ordinal an earlier runtime
// used: a seeded attempt never answers from an old attempt's record, and a
// pause an old runtime took is never passed as already taken. An acceptance
// round's call ids are `round * STALL_ATTEMPTS + 1` (authorOne).
function applySeedOrdinals() {
  const seed = phaseSeed();
  if (!seed) return;
  for (const [subject, ordinal] of Object.entries(seed.author_ordinals || {})) {
    const calls = subject === "acceptance" ? Math.ceil((ordinal - 1) / STALL_ATTEMPTS) : ordinal;
    AUTHOR_CALLS.set(subject, Math.max(AUTHOR_CALLS.get(subject) || 0, calls));
  }
  for (const [subject, ordinal] of Object.entries(seed.pause_ordinals || {})) {
    PAUSES.set(subject, Math.max(PAUSES.get(subject) || 0, ordinal));
  }
}

// The seed of `policy`'s subject as `{ feedback, carried }`, or null. `state`
// is the acceptance loop's entry state, filled here.
function seedAuthorLoop(policy, state) {
  const seed = phaseSeed();
  const subject = seed && seed.subjects[policy.phase];
  if (!subject || SEEDED_SUBJECTS.has(policy.phase)) return null;
  SEEDED_SUBJECTS.add(policy.phase);
  if (subject.kind === "entries" && policy.author) return seedEntries(subject, policy, state);
  if (subject.kind === "artifact" && !policy.author) return seedArtifact(subject, policy);
  throw new Error(`phase seed for ${policy.phase} is of kind ${subject.kind}, which its author loop does not take`);
}

function seedRepairFindings(gate, policy) {
  return (Array.isArray(gate.findings) ? gate.findings : []).filter((finding) => policy.retryScopes.has(finding.remediation_scope));
}

function seedEntries(seed, policy, state) {
  const ids = new Set(Object.keys(args.acceptanceCriteria || {}));
  const candidate = typeof seed.candidate === "string" ? JSON.parse(seed.candidate) : null;
  const gates = Array.isArray(seed.gates) ? seed.gates : [];
  // Every gate registers the supplementary checks it says are owed, in order,
  // as the loop did; the last one also names the entries it sent back.
  // `null` from the last gate: a finding no rule attributes to one entry, which
  // the live loop answers by authoring every entry again.
  let refuted = new Set();
  gates.forEach((gate, index) => {
    const last = index === gates.length - 1;
    const named = seedRepairIds(seedRepairFindings(gate, policy), ids, gate.published === true, last ? candidate : null);
    if (last) refuted = named;
  });
  const listed = candidate ? [...(candidate.entries || []), ...(candidate.supplementary || [])] : [];
  for (const entry of listed) state.entries.set(entry.id, entry);
  // A reply authored after that gate is the entry's latest version, and a
  // repair only when it differs from what the gate judged: a gate asked the
  // same candidate again answers from its record, which keeps its old time.
  const repaired = new Set();
  const unreadable = [];
  for (const reply of Array.isArray(seed.replies) ? seed.replies : []) {
    let entry = null;
    try { entry = unwrapEntry(JSON.parse(reply.text), reply.id); } catch (_) { entry = null; }
    if (!entry || entry.id !== reply.id) { unreadable.push(reply.id); continue; }
    const judged = state.entries.get(reply.id);
    seedOwe(entry);
    if (!judged || acceptanceEntryKey(judged) !== acceptanceEntryKey(entry)) repaired.add(reply.id);
    state.entries.set(reply.id, entry);
  }
  const invalid = seed.invalid && typeof seed.invalid === "object" ? seed.invalid : {};
  // Refuted (every carried entry, for an unattributable refusal) and not
  // repaired since: that round's work is kept, the rest is authored again, so
  // the refused candidate is never submitted unchanged.
  const sentBack = refuted === null ? listed.map((entry) => entry.id) : [...refuted];
  state.retryIds = new Set([...sentBack.filter((id) => !repaired.has(id)), ...Object.keys(invalid), ...unreadable]);
  const last = gates[gates.length - 1];
  const feedback = [
    ...(last ? seedRepairFindings(last, policy).map((finding) => String(finding.text)) : []),
    ...Object.values(invalid).flat().map(String)
  ];
  return { feedback, carried: null };
}

// A shape refusal names its entry by a pointer into the refused candidate's
// own lists (`supplementary/0/criterion ...`); resolved against that
// candidate, it sends back the one entry. Every other finding is read as the
// loop reads it (acceptanceRepairIds): one no rule attributes to an entry
// returns null, "every entry", exactly as the live loop reads it.
const SEED_POINTER = /^(?:candidate artifact (?:was refused|rejected):\s*)?\/?(entries|supplementary)\/(0|[1-9]\d*)(?=[/\s]|$)/;

function seedRepairIds(findings, ids, published, candidate) {
  const named = new Set();
  const rest = [];
  for (const finding of findings) {
    const match = SEED_POINTER.exec(String(finding.text || ""));
    const list = match && candidate && Array.isArray(candidate[match[1]]) ? candidate[match[1]] : null;
    const entry = list ? list[Number(match[2])] : null;
    if (entry && typeof entry.id === "string") named.add(entry.id);
    else rest.push(finding);
  }
  const attributed = acceptanceRepairIds(rest, ids, published, candidate);
  if (attributed === null) return null;
  for (const id of attributed) named.add(id);
  return named;
}

// The host-owned fields of an owed supplementary check, as the author step
// sets them before it keeps an entry.
function seedOwe(entry) {
  const sup = owedSupplementary().get(entry.id);
  if (!sup) return entry;
  const covers = Array.isArray(entry.covers) ? entry.covers.filter((c) => typeof c === "string") : [];
  entry.covers = [sup.requirement, ...covers.filter((c) => c !== sup.requirement)];
  entry.gap_permitted = false;
  return entry;
}

function seedArtifact(seed, policy) {
  if (typeof seed.candidate !== "string" || seed.candidate.length === 0) {
    throw new Error(`phase seed for ${policy.phase} carries no candidate`);
  }
  const repair = seed.gate ? seedRepairFindings(seed.gate, policy).map((finding) => String(finding.text)) : [];
  return repair.length > 0 ? { feedback: repair, carried: null } : { feedback: [], carried: seed.candidate };
}
