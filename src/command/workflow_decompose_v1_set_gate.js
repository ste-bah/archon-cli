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
// names, else to the skeleton. Rounds follow the one progress rule every
// author loop follows (workflow_decompose_v1_progress.js, Issue 261): a round
// makes progress when its distinct defect count sets a new best, a
// round of SET_GATE_ESCALATE_AFTER without progress escalates the repair to a
// skeleton re-author with every open finding, and STALL_ATTEMPTS rounds in a
// row without progress PAUSE the run with the findings listed. A resumed run
// starts a fresh window and may escalate again.
const SET_GATE_ESCALATE_AFTER = 2;

// How many authors run at once: the run's max parallelism, which the host
// derives from `subagent.max_concurrent` and also writes into the spec as
// `max_parallelism`. One source for the acceptance entries and the bodies.
function authorBatchSize() {
  return Number.isSafeInteger(args.authorMaxParallelism) && args.authorMaxParallelism > 0
    ? args.authorMaxParallelism : 1;
}

// Runs `launch(index, settled)` for index 0..count-1 with at most `cap` in
// flight, starting indices in order (Issue-247). Batches of `cap` left a freed
// slot idle until the slowest call of its batch ended: live, 15-29 minutes per
// slot on hour-long author calls.
// - window false: a plain pool; the next index starts as soon as any call ends.
// - window true: index i starts only once every index <= i - cap has settled,
//   so `settled[0..i-cap]` is fixed when i starts and anything i reads from it
//   depends on its index, never on which sibling happened to finish first.
//   A slow call delays only the calls behind it, not a whole batch.
// After a failure (a rejection, or a value `failed` accepts) nothing new
// starts, and every started call settles before this returns: a sibling agent
// is never abandoned mid-call. An index already queued when a failure lands
// is checked again just before its launch and skipped. `settled[i]` is
// `{status, value|reason}` for a launched index and undefined for one never
// launched; launched indices are always a prefix, `started` long.
function runBounded(count, cap, window, launch, failed = () => false) {
  return new Promise((resolve) => {
    const settled = new Array(count);
    let next = 0;
    let started = 0;
    let running = 0;
    let prefix = 0;
    let stopped = false;
    const finish = () => {
      if (running === 0 && (stopped || next >= count)) resolve({ settled, started });
    };
    const settle = (index, outcome) => {
      running -= 1;
      if (outcome) {
        settled[index] = outcome;
        try {
          if (outcome.status === "rejected" || failed(outcome.value)) stopped = true;
        } catch (reason) {
          // A predicate that throws must not strand the pool unresolved.
          settled[index] = { status: "rejected", reason };
          stopped = true;
        }
      }
      while (prefix < count && settled[prefix]) prefix += 1;
      pump();
      finish();
    };
    const pump = () => {
      while (!stopped && next < count && running < cap && (!window || next < prefix + cap)) {
        const index = next++;
        running += 1;
        Promise.resolve()
          .then(() => {
            if (stopped) return null;
            started += 1;
            try {
              return Promise.resolve(launch(index, settled))
                .then((value) => ({ status: "fulfilled", value }), (reason) => ({ status: "rejected", reason }));
            } catch (reason) {
              return { status: "rejected", reason };
            }
          })
          .then((outcome) => settle(index, outcome));
      }
    };
    pump();
    finish();
  });
}

// Bodies are authored up to `authorBatchSize()` at a time in a plain pool. A
// body needs only the frozen skeleton, the PRD and the repository, so sibling
// bodies are independent and cross-body consistency stays the set gate's job.
// `work` is `[subject, initialFeedback]` pairs in skeleton order; results
// enter `bodies` in that order once every started call has settled, so the Map
// and the evidence built from it read as the sequential loop's did. The first
// failure in skeleton order (lowest index, not first to land) is raised
// after all started calls settle. A body fails only by throwing, so there is
// no failed value for a rejection to outrank.
async function authorBodies(w, work, bodies) {
  const { settled } = await runBounded(work.length, authorBatchSize(), false,
    (index) => authorCandidate(w, bodyPolicy(work[index][0], work[index][1])));
  settled.forEach((result, index) => {
    if (result && result.status === "fulfilled") bodies.set(work[index][0].fileName, result.value);
  });
  const failure = settled.find((result) => result && result.status === "rejected");
  if (failure) throw failure.reason;
}

// `chain` is the frozen skeleton this run stands on: `{ outcome, subjects }`,
// replaced whenever the loop re-freezes the skeleton.
async function runSetGateLoop(w, chain, bodies) {
  const progress = newProgress([]);
  let escalated = false;
  for (let round = 1; ; round += 1) {
    const taskSetLint = await runSetGate(w, "task-set-lint");
    const requirementsTrace = await runSetGate(w, "requirements-trace");
    const gates = [taskSetLint, requirementsTrace];
    progress.calls = round;
    const failed = gates.find((gate) => gate.routed.operational);
    if (failed) {
      // A gate that could not run judged nothing: a round without progress.
      const summary = `${failed.capability}: ${failed.routed.operational}`;
      recordOperational(progress, round, summary);
      const stall = stallReason(progress);
      if (stall) await pauseAuthorLoop(w, "set-gates", progress, stall, [`host gate operational failure: ${summary}`], { rounds: round });
      continue;
    }
    const open = gates.flatMap((gate) => gate.routed.all);
    if (open.length === 0) {
      return {
        taskSetLint: acceptSetGate(taskSetLint),
        requirementsTrace: acceptSetGate(requirementsTrace)
      };
    }
    if (recordAttempt(progress, round, gates.flatMap((gate) => gate.routed.allFindings))) escalated = false;
    const bodyFindings = gates.flatMap((gate) => gate.routed.retryFindings);
    const skeletonFindings = gates.flatMap((gate) => gate.routed.shadowFindings);
    for (const finding of gates.flatMap((gate) => gate.routed.inheritedFindings)) {
      // The predecessor the finding names repairs it; one that names no
      // frozen task is the skeleton's to repair.
      (findSubject(finding, chain.subjects) ? bodyFindings : skeletonFindings).push(finding);
    }
    const stall = stallReason(progress);
    if (stall) {
      // Resumed past the pause: a fresh window, which repairs before it judges.
      await pauseAuthorLoop(w, "set-gates", progress, stall, open, { rounds: round });
      escalated = false;
    }
    if (!escalated && progress.stalled >= SET_GATE_ESCALATE_AFTER) {
      escalated = true;
      await reauthorSkeleton(w, chain, bodies, gates.flatMap((gate) => gate.routed.allFindings), bodyFindings);
      continue;
    }
    if (skeletonFindings.length > 0) {
      await reauthorSkeleton(w, chain, bodies, skeletonFindings, bodyFindings);
      continue;
    }
    // Re-authored the way they were authored: in the bounded pool, each body alone
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
// A verification that could not run is retried, and pauses the run when its
// window closes, like any other attempt without progress.
async function verifyFrozenStage(w, capability) {
  const progress = newProgress([]);
  for (let attempt = 1; ; attempt += 1) {
    const outcome = await w.hostCommand(capability, { stdin: null });
    const routed = routeFindings(outcome, new Set(), new Set());
    if (!routed.operational) {
      if (routed.fatal.length > 0) await stopFixed(`${capability} stopped: ${routed.fatal.join(" | ")}`);
      requireCommitted(outcome, capability);
      return outcome;
    }
    progress.calls = attempt;
    recordOperational(progress, attempt, routed.operational);
    const stall = stallReason(progress);
    if (stall) await pauseAuthorLoop(w, capability, progress, stall, [`host gate operational failure: ${routed.operational}`]);
  }
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
