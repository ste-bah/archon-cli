// Set-gate repair loop for the fixed decomposition script (Issue-46).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// Each body is judged alone, against the obligations it claims, while its
// author still has attempts. A hole one body opens in another's claim is
// only visible once the whole set exists, so the set gate is the first place
// it can be found — and it used to be fatal there. Live: eight hours of
// authoring ended on one such finding, and runs are pinned to their starting
// binary, so nothing could pick the set back up. The gate now sends the
// finding to the body it names, re-authors that body with the finding as its
// opening feedback, and asks both gates again.
//
// Batch O: the loop never accepts a set with findings open, in either gate
// mode. A skeleton finding (an obligation no task claims, a claim of an id
// the PRD does not define) re-authors the skeleton with the finding -- an
// owning task's implements gains the id -- re-freezes it, and re-authors the
// bodies against it; an inherited finding goes to the predecessor body it
// names, else to the skeleton. The budget follows progress: rounds continue
// while the open-finding count keeps falling below its best, a plateau of
// SET_GATE_STALL_ROUNDS escalates to a skeleton re-author with every open
// finding, and only a plateau after that escalation stops the run, with the
// findings listed.
const SET_GATE_STALL_ROUNDS = 2;

// How many authors run at once: the run's max parallelism, which the host
// derives from `subagent.max_concurrent` and also writes into the spec as
// `max_parallelism`. One source for the acceptance entries and the bodies.
function authorBatchSize() {
  return Number.isSafeInteger(args.authorMaxParallelism) && args.authorMaxParallelism > 0
    ? args.authorMaxParallelism : 1;
}

// Bodies are authored `authorBatchSize()` at a time. A body needs only the
// frozen skeleton, the PRD and the repository, so sibling bodies are
// independent and cross-body consistency stays the set gate's job; authoring
// them one after another spent hours of wall clock on work that does not
// wait on itself. `work` is `[subject, initialFeedback]` pairs in skeleton
// order; results enter `bodies` in that order after each batch, so the Map
// and the evidence built from it read as the sequential loop's did. Every
// started call settles before the next batch or a failure is raised: a
// sibling agent is never abandoned mid-call.
async function authorBodies(w, work, bodies) {
  const cap = authorBatchSize();
  for (let start = 0; start < work.length; start += cap) {
    const batch = work.slice(start, start + cap);
    const settled = await Promise.allSettled(
      batch.map(([subject, feedback]) => authorCandidate(w, bodyPolicy(subject, feedback)))
    );
    settled.forEach((result, index) => {
      if (result.status === "fulfilled") bodies.set(batch[index][0].fileName, result.value);
    });
    const failure = settled.find((result) => result.status === "rejected");
    if (failure) throw failure.reason;
  }
}

// `chain` is the frozen skeleton this run stands on: `{ outcome, subjects }`,
// replaced whenever the loop re-freezes the skeleton.
async function runSetGateLoop(w, chain, bodies) {
  let best = Infinity;
  let stalled = 0;
  let escalated = false;
  for (let round = 1; ; round += 1) {
    const taskSetLint = await runSetGate(w, "task-set-lint");
    const requirementsTrace = await runSetGate(w, "requirements-trace");
    const gates = [taskSetLint, requirementsTrace];
    const open = gates.flatMap((gate) => gate.routed.all);
    if (open.length === 0) {
      return {
        taskSetLint: acceptSetGate(taskSetLint),
        requirementsTrace: acceptSetGate(requirementsTrace)
      };
    }
    if (open.length < best) {
      best = open.length;
      stalled = 0;
      escalated = false;
    } else {
      stalled += 1;
    }
    const bodyFindings = gates.flatMap((gate) => gate.routed.retryFindings);
    const skeletonFindings = gates.flatMap((gate) => gate.routed.shadowFindings);
    for (const finding of gates.flatMap((gate) => gate.routed.inheritedFindings)) {
      // The predecessor the finding names repairs it; one that names no
      // frozen task is the skeleton's to repair.
      (findSubject(finding, chain.subjects) ? bodyFindings : skeletonFindings).push(finding);
    }
    if (stalled >= SET_GATE_STALL_ROUNDS) {
      if (escalated) {
        throw new Error(`set gates made no progress in ${round} rounds, a skeleton re-author included, with findings still open: ${open.join(" | ")}`);
      }
      escalated = true;
      stalled = 0;
      await reauthorSkeleton(w, chain, bodies, gates.flatMap((gate) => gate.routed.allFindings), bodyFindings);
      continue;
    }
    if (skeletonFindings.length > 0) {
      await reauthorSkeleton(w, chain, bodies, skeletonFindings, bodyFindings);
      continue;
    }
    // Re-authored the way they were authored: in batches, each body alone
    // with its own findings.
    const work = [];
    for (const [fileName, findings] of groupFindingsBySubject(bodyFindings, chain.subjects)) {
      const subject = chain.subjects.find((candidate) => candidate.fileName === fileName);
      work.push([subject, findings.map((finding) => findingText(finding))]);
    }
    await authorBodies(w, work, bodies);
  }
}

// Re-author and re-freeze the skeleton with `findings` as its opening
// feedback, then re-author every body against it: each copies its frozen
// fields from the skeleton, which just changed. A re-frozen skeleton keeps
// every frozen task; a task it adds is authored like any other.
async function reauthorSkeleton(w, chain, bodies, findings, bodyFindings) {
  const outcome = await authorCandidate(w, skeletonPolicy(findings.map((finding) => findingText(finding))));
  if (!Array.isArray(outcome.subjects) || outcome.subjects.length === 0) {
    throw new Error("re-frozen skeleton returned zero host-read task subjects");
  }
  for (const subject of outcome.subjects) requireSubject(subject);
  for (const kept of chain.subjects) {
    if (!outcome.subjects.some((subject) => subject.taskId === kept.taskId && subject.fileName === kept.fileName)) {
      throw new Error(`re-frozen skeleton dropped frozen task ${kept.taskId} (${kept.fileName}); a re-author may add implements and tasks, never remove one`);
    }
  }
  chain.outcome = outcome;
  chain.subjects = outcome.subjects;
  const grouped = new Map(groupFindingsBySubject(bodyFindings, chain.subjects));
  const note = "The frozen skeleton was re-authored to repair set-gate findings; copy this task's frozen fields from the current skeleton exactly.";
  await authorBodies(w, chain.subjects.map((subject) => [
    subject,
    [note, ...(grouped.get(subject.fileName) || []).map((finding) => findingText(finding))]
  ]), bodies);
}

function committed(outcome) {
  return Boolean(outcome && outcome.publicationReceipt && outcome.postcondition?.satisfied === true);
}

// Findings keyed by the frozen file name of the task each one names, in
// subject order. A finding that names no frozen task stops the run: silently
// dropping it would accept a set the gate refused.
function groupFindingsBySubject(findings, subjects) {
  const grouped = new Map();
  for (const finding of findings) {
    const subject = resolveFindingSubject(finding, subjects);
    if (!grouped.has(subject.fileName)) grouped.set(subject.fileName, []);
    grouped.get(subject.fileName).push(finding);
  }
  return [...grouped.entries()].sort(
    ([a], [b]) => subjects.findIndex((s) => s.fileName === a) - subjects.findIndex((s) => s.fileName === b)
  );
}

// The gate names the weakest task by its file path first. A finding without
// a resolvable path falls back to the `task TASK-…:` quote in its text, then
// to its `subject` field; nothing else is guessed.
function resolveFindingSubject(finding, subjects) {
  const subject = findSubject(finding, subjects);
  if (subject) return subject;
  const path = typeof finding?.source_path === "string" ? finding.source_path : "";
  throw new Error(`set gate finding names no frozen task (source_path=${path || "none"}, subject=${finding?.subject ?? "none"}): ${findingText(finding)}`);
}

// The frozen subject a finding names, or null.
function findSubject(finding, subjects) {
  const path = typeof finding?.source_path === "string" ? finding.source_path : "";
  const baseName = path.split(/[\\/]/).pop();
  if (baseName) {
    const byFile = subjects.find((subject) => subject.fileName === baseName);
    if (byFile) return byFile;
  }
  const text = findingText(finding);
  const quoted = text.match(/\btask\s+([A-Za-z0-9][A-Za-z0-9_.-]*):/);
  if (quoted) {
    const byQuote = subjects.find((subject) => subject.taskId === quoted[1]);
    if (byQuote) return byQuote;
  }
  if (typeof finding?.subject === "string") {
    const bySubject = subjects.find((subject) => subject.taskId === finding.subject);
    if (bySubject) return bySubject;
  }
  return null;
}

function findingText(finding) {
  return typeof finding?.text === "string" ? finding.text : "unnamed policy finding";
}

// One evidence entry per authored subject, in frozen order.
function bodyEvidence(subjects, bodies) {
  const evidence = [];
  for (const subject of subjects) {
    if (bodies.has(subject.fileName)) evidence.push(bodies.get(subject.fileName));
  }
  return evidence;
}

// The launcher's reading of the task root: which frozen stages exist and
// verify, and which frozen bodies are already on disk. Absent means nothing
// is frozen. A skeleton without an acceptance contract, or bodies without a
// skeleton, is a chain the host would never have reported.
function frozenChain() {
  const raw = args.frozenChain;
  const none = { acceptance: false, skeleton: false, subjects: [], bodies: new Set() };
  if (raw === undefined || raw === null) return none;
  if (typeof raw !== "object" || Array.isArray(raw)) throw new Error("fixed decomposition frozenChain argument is malformed");
  const acceptance = raw.acceptance === true;
  const skeleton = raw.skeleton === true;
  const subjects = Array.isArray(raw.subjects) ? raw.subjects : [];
  const bodies = new Set(Array.isArray(raw.bodies) ? raw.bodies : []);
  if (skeleton && !acceptance) throw new Error("frozenChain reports a skeleton without an acceptance contract");
  if (!skeleton && (subjects.length > 0 || bodies.size > 0)) throw new Error("frozenChain reports subjects or bodies without a frozen skeleton");
  for (const subject of subjects) requireSubject(subject);
  for (const fileName of bodies) {
    if (!subjects.some((subject) => subject.fileName === fileName)) throw new Error(`frozenChain body ${fileName} names no frozen subject`);
  }
  return { acceptance, skeleton, subjects, bodies };
}

// A committed outcome for a stage the launcher found frozen: the host
// re-verifies the artifact in place and reports its subjects as the freeze
// would have. Any finding here is fatal; there is no candidate to repair.
async function verifyFrozenStage(w, capability) {
  const outcome = await w.hostCommand(capability, { stdin: null });
  const routed = routeFindings(outcome, new Set(), new Set());
  if (routed.fatal.length > 0) throw new Error(`${capability} stopped: ${routed.fatal.join(" | ")}`);
  requireCommitted(outcome, capability);
  return outcome;
}

// The subjects the host read at verification are the ones the launcher read,
// or the skeleton was re-frozen underneath the run. A rehearsal answers with
// a stand-in subject and is not held to this.
function requireFrozenSubjects(frozen, skeleton) {
  if (!frozen.skeleton || skeleton.dryRun === true) return;
  const key = (subject) => `${subject.taskId}\u0000${subject.fileName}`;
  const launched = frozen.subjects.map(key).sort();
  const verified = skeleton.subjects.map(key).sort();
  if (launched.length !== verified.length || launched.some((entry, index) => entry !== verified[index])) {
    throw new Error("frozen skeleton subjects differ from the launch-bound frozenChain; the task root changed underneath the run");
  }
}
