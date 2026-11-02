// The failing-subject side of an attempt's progress (Issues 529, 561), and its
// gate-stage side (Issue 562).
// Loaded after workflow_decompose_v1_progress.js as one script: every
// declaration here is hoisted into the same scope as `workflow`.

// The subject a finding names (its host defect's, else its own), or "" when
// it names none. Never its text.
function namedSubject(finding) {
  const defect = finding && finding.deterministic_defect;
  return defect && typeof defect.subject === "string" && defect.subject.length > 0
    ? defect.subject : finding && typeof finding.subject === "string" && finding.subject.length > 0
      ? finding.subject : "";
}

function failingSubjectKey(finding) {
  const subject = namedSubject(finding);
  return subject === "" ? findingKey(finding) : subject;
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

// Issue 562: the gate stages that produce findings, in pipeline order; mirrors
// the host's GateStage (a test fails if they drift). A finding that names no
// stage comes from the gate's own validators, or from an envelope written
// before findings carried a stage.
const GATE_STAGES = ["judge", "probe", "passability", "critic"];

// How far through its gate a finding got: the stage that produced it, then a
// host defect's validation stage, with a judged finding above every
// validation stage. Host-owned fields only, never text.
function gateStageRank(finding) {
  const named = finding && typeof finding === "object" ? GATE_STAGES.indexOf(finding.gate_stage) : -1;
  return (named + 1) * (PASSED_STAGE + 1) + (isDeterministic(finding) ? stageOf(finding) : PASSED_STAGE);
}

// One open finding: its subject and stage, and what the host itself names
// within them (a defect's code and slot, or the item a judge or critic
// decided). Never its text, so rewording a judged finding changes nothing.
function openFindingKey(finding) {
  const defect = hostDefect(finding);
  const item = finding && typeof finding.judged_item === "string" ? finding.judged_item : "";
  return JSON.stringify([namedSubject(finding), gateStageRank(finding),
    defect ? defect.code : "", defect ? String(defect.location || "") : "", item]);
}

// How far the gate evaluated an attempt, for counting its open findings: its
// tier, then its first failing validation stage up to the contract checks.
// Earlier stages can stop the gate before later ones report, as a lower tier
// does, so fewer findings there is not less open work; contract checks and
// judged findings are reported per subject beside each other.
const CONTRACTS_STAGE = DEFECT_STAGES.indexOf("contracts");
function evaluationLevel(measure) {
  return measure.tier * (CONTRACTS_STAGE + 1) + Math.min(measure.stage, CONTRACTS_STAGE);
}

// Issue 562: the other two kinds of progress, measured against ratchets that
// only improve (best-so-far, never the previous attempt alone):
// `fewer`: fewer distinct open findings (openFindingKey) than the fewest of
// any earlier attempt the gate evaluated as far (evaluationLevel); an attempt
// it evaluated less far is not counted, and one it evaluated further starts
// the count again (that attempt has a better measure, so it is progress).
// `advanced`: the subjects whose earliest open finding (lowest gateStageRank)
// is at a later stage than their best earliest so far, credited only when the
// whole attempt is a Pareto improvement on the previous attempt: it names no
// subject the previous one did not, and no subject's earliest or furthest
// open finding (highest rank) is at an earlier stage than in the previous
// attempt, and every open finding (openFindingKey) the previous attempt did
// not have belongs to a subject whose earliest stage rose over that attempt.
// Findings are not matched across attempts, so one subject's stages can rise
// only because an early failure moved to another subject; that subject then
// regresses, is new, or gains a finding without advancing, and the attempt is
// not credited. A subject whose one finding moves judge, probe, passability
// still advances, and may gain the new finding of its later stage.
// A subject first seen has no best, so its first sighting is never progress,
// and only an eligible subject is credited: one that failed in an attempt
// credited by the measure, the baseline subset or `fewer`.
// Bounded: there are finitely many levels and the level only rises; at one
// level the fewest count is a natural number that every `fewer` lowers. The eligible
// subjects are the union of the sets of finitely many credited attempts (the
// measure and subset rules are finite, see failingSetStep), so they are
// finite, and every `advanced` raises one eligible subject's best earliest
// rank among finitely many (both bests only rise); no credit here resets the baseline or adds an eligible
// subject of its own. So these rules credit finitely often, and between two
// credits STALL_ATTEMPTS attempts that add to the window pause the run. A
// repeated set of (subject, stage) pairs, a trade at the same count, or an
// oscillation between two states improves neither ratchet after its first
// visit, so it fills the window.
function gateStageStep(progress, findings, measure, better) {
  if (!(progress.stageBest instanceof Map)) progress.stageBest = new Map();
  if (!(progress.stageEligible instanceof Set)) progress.stageEligible = new Set();
  const open = new Set(), stages = new Map(), keySubjects = new Map();
  for (const finding of findings) {
    const rank = gateStageRank(finding), subject = namedSubject(finding), key = openFindingKey(finding);
    open.add(key);
    keySubjects.set(key, subject);
    const seen = stages.get(subject);
    if (subject !== "") stages.set(subject, seen ? [Math.min(seen[0], rank), Math.max(seen[1], rank)] : [rank, rank]);
  }
  const level = evaluationLevel(measure);
  const further = !Number.isInteger(progress.bestOpen) || level > progress.bestOpenLevel;
  const fewer = level === progress.bestOpenLevel && open.size < progress.bestOpen;
  if (further || fewer) [progress.bestOpen, progress.bestOpenLevel] = [open.size, level];
  const previous = progress.lastStages instanceof Map ? progress.lastStages : null;
  const lastOpen = progress.lastOpen instanceof Set ? progress.lastOpen : new Set();
  const rose = subject => previous !== null && previous.has(subject) && stages.get(subject)[0] > previous.get(subject)[0];
  const pareto = previous !== null && [...stages].every(([subject, [earliest, furthest]]) =>
    previous.has(subject) && earliest >= previous.get(subject)[0] && furthest >= previous.get(subject)[1])
    && [...open].every(key => lastOpen.has(key) || (keySubjects.get(key) !== "" && rose(keySubjects.get(key))));
  const advanced = [];
  // Each subject's best is [earliest, furthest], each the highest seen so far.
  for (const [subject, [earliest, furthest]] of stages) {
    const best = progress.stageBest.get(subject);
    if (pareto && best && earliest > best[0] && progress.stageEligible.has(subject)) advanced.push(subject);
    progress.stageBest.set(subject, best ? [Math.max(best[0], earliest), Math.max(best[1], furthest)] : [earliest, furthest]);
  }
  [progress.lastStages, progress.lastOpen] = [stages, open];
  if (better || fewer) for (const subject of stages.keys()) progress.stageEligible.add(subject);
  return { fewer, advanced };
}

// A gate stage rank as its name, for evidence: "validation" or a GATE_STAGES
// name, then the validation stage for a host defect.
function gateStageName(rank) {
  const gate = Math.floor(rank / (PASSED_STAGE + 1)), within = rank % (PASSED_STAGE + 1);
  return `${gate === 0 ? "validation" : GATE_STAGES[gate - 1]}:${DEFECT_STAGES[within] || "judged"}`;
}

// The #562 ratchets a pause carries, bounded like its other evidence.
function gateStageEvidence(progress) {
  if (!Number.isInteger(progress.bestOpen)) return {};
  const stages = progress.stageBest instanceof Map ? [...progress.stageBest] : [];
  return {
    best_open_findings: progress.bestOpen,
    subject_stages: stages.slice(0, PAUSE_EVIDENCE_FINDINGS).map(([subject, [earliest]]) => [boundText(subject), gateStageName(earliest)]),
    subject_stages_total: stages.length
  };
}
