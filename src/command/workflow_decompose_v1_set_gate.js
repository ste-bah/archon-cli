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
// skeleton re-author with skeleton findings, and STALL_ATTEMPTS rounds
// without progress advance the authored repair rung.
const SET_GATE_ESCALATE_AFTER = 2;

// How many authors run at once: the run's max parallelism, which the host
// derives from `subagent.max_concurrent` and also writes into the spec as
// `max_parallelism`. One source for the acceptance entries and the bodies.
function authorBatchSize() {
  return Number.isSafeInteger(args.authorMaxParallelism) && args.authorMaxParallelism > 0
    ? args.authorMaxParallelism : 1;
}

// Runs `launch(index)` for index 0..count-1 with at most `cap` in
// flight, starting indices in order (Issue-247). Batches of `cap` left a freed
// slot idle until the slowest call of its batch ended: live, 15-29 minutes per
// slot on hour-long author calls.
// The next index starts as soon as any call ends. After a rejection or a
// value `failed` accepts, nothing new starts. Which siblings started before
// that outage settled can depend on timing; no prompt may depend on sibling
// completion order. Callers must use inputs recorded before fan-out and retain
// outage results by index, not by which calls happened to start. Every started
// call settles before this returns: a sibling agent is never abandoned
// mid-call. An index already queued when a rejection lands is checked again
// just before its launch and skipped. `settled[i]` is `{status,
// value|reason}` for a launched index and undefined for one never launched;
// launched indices are always a prefix, `started` long.
function runBounded(count, cap, launch, failed = () => false) {
  return new Promise((resolve) => {
    const settled = new Array(count);
    let next = 0;
    let started = 0;
    let running = 0;
    let stopped = false;
    const open = () => !stopped && next < count;
    const finish = () => {
      if (running === 0 && !open()) resolve({ settled, started });
    };
    const settle = (index, outcome) => {
      running -= 1;
      if (outcome) {
        settled[index] = outcome;
        try {
          if (outcome.status === "rejected") stopped = true;
          else if (failed(outcome.value)) stopped = true;
        } catch (reason) {
          // A predicate that throws must not strand the pool unresolved.
          settled[index] = { status: "rejected", reason };
          stopped = true;
        }
      }
      pump();
      finish();
    };
    const pump = () => {
      while (open() && running < cap) {
        const index = next++;
        running += 1;
        Promise.resolve()
          .then(() => {
            if (stopped) return null;
            started += 1;
            try {
              return Promise.resolve(launch(index))
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
async function authorBodies(w, work, bodies, chain) {
  const skeleton = chain?.outcome;
  const { settled } = await runBounded(work.length, authorBatchSize(),
    (index) => {
      const policy = bodyPolicy(work[index][0], work[index][1]);
      policy.deferAuthoredPending = true;
      if (Number.isInteger(work[index][2])) policy.initialRung = work[index][2];
      return authorCandidate(w, policy);
    });
  const pending = [];
  settled.forEach((result, index) => {
    if (result && result.status === "fulfilled") {
      if (result.value?.pendingAuthor) pending.push([work[index][0], result.value.findings, work[index][1] || [], result.value.pendingRung]);
      else if (!result.value?.skeletonFindings) bodies.set(work[index][0].fileName, result.value);
    }
  });
  const failure = settled.find((result) => result && result.status === "rejected");
  if (failure) throw failure.reason;
  if (pending.length > 0) {
    await pauseLoop(w, "authored-pending", {
      reason: "authored_pending",
      pending_subjects: pending.map(([subject]) => subject.taskId),
      last_findings: pending.flatMap(([, findings]) => findings).slice(0, PAUSE_EVIDENCE_FINDINGS),
      recovery: "The run is paused with authored subjects pending. Resume to retry these subjects; completed independent bodies remain published."
    });
    await authorBodies(w, pending.map(([subject, findings, priorFeedback, rung]) => [
      subject, [...priorFeedback, ...findings], rung
    ]), bodies, chain);
  }
  const shadows = settled.filter((r) => r?.status === "fulfilled" && r.value?.skeletonFindings);
  if (shadows.length > 0) {
    // A pending sibling may have repaired the skeleton. Recheck these bodies
    // against that skeleton before acting on findings from its predecessor.
    if (chain.outcome !== skeleton) return await authorBodies(w, work.flatMap(([subject, feedback, rung], i) =>
      settled[i]?.value?.skeletonFindings
        ? [[chain.subjects.find((s) => s.taskId === subject.taskId), feedback, rung]] : []), bodies, chain);
    const repairBodies = settled.flatMap((r, index) => r?.status === "fulfilled" && r.value?.skeletonFindings
      ? [...r.value.bodyFindings, { text: BODY_FROZEN_RULE, subject: work[index][0].taskId }] : []);
    const findings = shadows.flatMap((r) => r.value.skeletonFindings);
    // Shadow repairs progress only when the open count beats its best.
    // Stable count slots keep text, subjects, locations and stages from
    // crediting a trade; recordAttempt owns the same stall window as elsewhere.
    const measured = findings.map((_, i) => ({ text: "skeleton shadow", subject: "body-skeleton",
      deterministic_defect: { provenance: "host_validator", code: "skeleton_shadow", location: String(i) } }));
    if (!chain.bodyShadowRepair) {
      const progress = newProgress([]);
      recordAttempt(progress, 0, measured, false);
      chain.bodyShadowRepair = { progress, rung: 1, cycle: 1 };
    }
    const repair = chain.bodyShadowRepair, progress = repair.progress;
    progress.calls += 1;
    recordAttempt(progress, progress.calls, measured);
    const stall = stallReason(progress);
    if (stall) await pauseAuthorLoop(w, "body-skeleton", progress, stall, findings.map(findingText));
    repair.rung = Math.max(repair.rung, ...settled.flatMap((r, i) => r?.value?.skeletonFindings ? [r.value.shadowRung || work[i][2] || 1] : []));
    repair.rung = repair.rung === 4 ? 2 : repair.rung + 1;
    if (repair.rung === 2) repair.cycle += 1;
    chain.bodyShadowRound = (chain.bodyShadowRound || 0) + 1;
    await repairEvent(w, `body-skeleton-${chain.bodyShadowRound}`, repair.rung, repair.cycle, progress.calls, progress.calls);
    await reauthorSkeleton(w, chain, bodies, findings,
      repairBodies, repair.rung);
    // Only the frame that observed shadows may finish their progress window;
    // a pending-only nested retry has no evidence that those shadows cleared.
    chain.bodyShadowRepair = null;
  }
}

// `chain` is the frozen skeleton this run stands on: `{ outcome, subjects }`,
// replaced whenever the loop re-freezes the skeleton.
async function runSetGateLoop(w, chain, bodies) {
  const progress = newProgress([]);
  let escalated = false;
  let repairRung = 1;
  let repairCycle = 1;
  for (let round = 1; ; round += 1) {
    const taskSetLint = await runSetGate(w, "task-set-lint", chain.subjects);
    const requirementsTrace = await runSetGate(w, "requirements-trace", chain.subjects);
    const gates = [taskSetLint, requirementsTrace];
    progress.calls = round;
    const failed = gates.find((gate) => gate.routed.operational);
    if (failed) {
      // A gate that could not run judged nothing: a round without progress.
      // A judge reply it could not use is the judge's own (D5): it counts in
      // the ordinary no-progress window, never the host's operational one.
      const authoredReply = failed.routed.operationalKind === "authored_judge_reply";
      if (!authoredReply) recordOperational(progress, round, failed.routed.operational, false, NO_ROUND, failed.capability);
      if (authoredReply) recordAnswered(progress, round, "authored_judge_reply", false, true, NO_ROUND);
      const stall = stallReason(progress);
      if (stall === "operational_no_progress") {
        await pauseAuthorLoop(w, "set-gates", progress, stall, [`${failed.capability}: ${failed.routed.operational}`], { rounds: round });
        continue;
      }
      if (stall) {
        await pauseAuthorLoop(w, "set-gates", progress, stall, [`${failed.capability}: ${failed.routed.operational}`], { rounds: round });
        repairRung = repairRung === 4 ? 2 : repairRung + 1;
        if (repairRung === 2) {
          repairCycle += 1;
        }
        await repairEvent(w, "set-gates", repairRung, repairCycle, round, progress.calls);
        progress.stalled = 0;
        progress.stalledOperational = 0;
        progress.repair.stalled = 0;
      }
      if (!stall && authoredReply) {
        repairRung = repairRung === 4 ? 2 : repairRung + 1;
        if (repairRung === 2) repairCycle += 1;
        await repairEvent(w, "set-gates", repairRung, repairCycle, round, progress.calls);
      }
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
    await routeUnattributed(w, bodyFindings, skeletonFindings, chain.subjects, round);
    const stall = stallReason(progress);
    if (stall) {
      await pauseAuthorLoop(w, "set-gates", progress, stall, open, { rounds: round });
      repairRung = repairRung === 4 ? 2 : repairRung + 1;
      if (repairRung === 2) {
        repairCycle += 1;
      }
      await repairEvent(w, "set-gates", repairRung, repairCycle, round, progress.calls);
      progress.stalled = 0;
      progress.stalledOperational = 0;
      progress.repair.stalled = 0;
      escalated = false;
    }
    if (!stall) {
      repairRung = repairRung === 4 ? 2 : repairRung + 1;
      if (repairRung === 2) repairCycle += 1;
      await repairEvent(w, "set-gates", repairRung, repairCycle, round, progress.calls);
    }
    if (!escalated && progress.stalled >= SET_GATE_ESCALATE_AFTER) {
      escalated = true;
      if (skeletonFindings.length > 0) {
        skeletonFindings.push({ text: `${bodyFindings.length} body findings remain open; they are re-authored by the bodies` });
      }
    }
    if (skeletonFindings.length > 0) {
      await reauthorSkeleton(w, chain, bodies, skeletonFindings, bodyFindings, repairRung);
      continue;
    }
    // Re-authored the way they were authored: in the bounded pool, each body alone
    // with its own findings.
    const work = [];
    for (const [fileName, findings] of groupFindingsBySubject(bodyFindings, chain.subjects)) {
      const subject = chain.subjects.find((candidate) => candidate.fileName === fileName);
      work.push([subject, findings.map((finding) => findingText(finding)), repairRung]);
    }
    await authorBodies(w, work, bodies, chain);
  }
}

// Re-author and re-freeze the skeleton with `findings` as its opening
// feedback, then re-author only the bodies the change reaches (O3, as an
// incremental build re-runs only the actions whose inputs changed): a task
// whose frozen fields differ from the skeleton its body was authored
// against, a task the skeleton added, and a body a finding names. Every
// other body keeps its published bytes, and the set gates that follow judge
// the whole set again, kept bodies included. A skeleton verified in place
// carries the frozen entries the launcher read, and a body the launcher
// found on disk is one on hand like a body this run authored. When either
// skeleton's frozen fields are unknown to the script (a launch argument
// without them, or a candidate it cannot read), every body is re-authored.
async function reauthorSkeleton(w, chain, bodies, findings, bodyFindings, repairRung) {
  await routeUnattributed(w, bodyFindings, findings, chain.subjects, "body");
  const prior = SKELETON_TASKS.get(chain.outcome);
  const outcome = await authorCandidate(w, skeletonPolicy(
    findings.map((finding) => findingText(finding)), repairRung, skeletonCandidateContent(chain.outcome)));
  if (!Array.isArray(outcome.subjects) || outcome.subjects.length === 0) {
    throw new Error("re-frozen skeleton returned zero host-read task subjects");
  }
  for (const subject of outcome.subjects) requireSubject(subject);
  // A contract assertion only: the host's freeze-skeleton refuses a
  // candidate that drops a frozen task before it publishes anything (O1),
  // returning that refusal to the skeleton author like any other finding.
  for (const kept of chain.subjects) {
    if (!outcome.subjects.some((subject) => subject.taskId === kept.taskId && subject.fileName === kept.fileName)) {
      throw new Error(`re-frozen skeleton dropped frozen task ${kept.taskId} (${kept.fileName}); a re-author may add implements and tasks, never remove one`);
    }
  }
  const priorHostStates = new Map(chain.subjects.map(s => [s.taskId, JSON.stringify([s.requirementStates || {}, s.hostRecordDigest])]));
  chain.outcome = outcome;
  chain.subjects = outcome.subjects;
  const next = SKELETON_TASKS.get(outcome);
  // Inferred body omissions may still help a body author after skeleton repair.
  // Deterministic skeleton defects describe frozen fields only their owner can fix.
  const bodyContext = [...bodyFindings, ...findings.filter(finding => !finding.deterministic_defect)];
  const grouped = new Map(groupFindingsBySubject(bodyContext, chain.subjects));
  const skeletonSubjects = new Set(groupFindingsBySubject(findings, chain.subjects).map(([file]) => file));
  const onDisk = frozenChain().bodies;
  const dependencies = changedDependencySubjects(prior, next);
  const reached = (subject) => priorHostStates.get(subject.taskId) !== JSON.stringify([subject.requirementStates || {}, subject.hostRecordDigest]) || dependencies.has(subject.taskId) || !prior || !next
    || !(bodies.has(subject.fileName) || onDisk.has(subject.fileName))
    || grouped.has(subject.fileName) || skeletonSubjects.has(subject.fileName) || !prior.has(subject.taskId)
    || prior.get(subject.taskId) !== next.get(subject.taskId);
  const note = BODY_FROZEN_RULE;
  await authorBodies(w, chain.subjects.filter(reached).map((subject) => [
    subject,
    [note, ...(grouped.get(subject.fileName) || []).map((finding) => findingText(finding))], repairRung
  ]), bodies, chain);
}

// Each committed skeleton outcome's frozen fields per task id, read from
// the candidate it froze, or for a skeleton verified in place from the
// launcher's frozenChain.tasks: the host stamps every body's frozen fields
// from them (BODY_SHAPE). Kept beside the outcome, never on it, so the
// evidence the run reports is the host's alone.
const SKELETON_TASKS = new WeakMap();
let CURRENT_SKELETON_TASKS = null;
const SKELETON_CANDIDATES = new WeakMap();

// Verification reports are snapshots or diagnostics, never authored candidates.
function skeletonCandidateContent(outcome) {
  return SKELETON_CANDIDATES.get(outcome);
}

function recordSkeletonTasks(outcome, content) {
  SKELETON_CANDIDATES.set(outcome, content);
  const tasks = skeletonCandidateTasks(content);
  if (tasks) {
    SKELETON_TASKS.set(outcome, tasks);
    CURRENT_SKELETON_TASKS = tasks;
  }
}

// `taskId -> canonical JSON of its frozen entry`, or null when `content`
// holds no skeleton the script can read.
function skeletonCandidateTasks(content) {
  const text = String(content || "").trim();
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end < start) return null;
  let skeleton;
  try {
    skeleton = JSON.parse(text.slice(start, end + 1));
  } catch (_) {
    return null;
  }
  return skeleton ? frozenTaskEntries(skeleton.tasks) : null;
}

// `taskId -> canonical JSON of its frozen entry`, or null when `tasks` is
// not a list of entries. A field at its default (null, false, 0, an empty
// list or object) reads as absent, as the host's skeleton reader takes it:
// the published form spells defaults out and an author's candidate may not.
function frozenTaskEntries(tasks) {
  if (!Array.isArray(tasks)) return null;
  const normal = (value) => {
    if (Array.isArray(value)) return value.map(normal);
    if (!value || typeof value !== "object") return value;
    const kept = {};
    for (const [key, field] of Object.entries(value)) {
      const read = normal(field);
      if (read === null || read === false || read === 0) continue;
      if (typeof read === "object" && Object.keys(read).length === 0) continue;
      kept[key] = read;
    }
    return kept;
  };
  const canonical = (value) => Array.isArray(value)
    ? `[${value.map(canonical).join(",")}]`
    : value && typeof value === "object"
      ? `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`
      : JSON.stringify(value);
  const entries = new Map();
  for (const entry of tasks) {
    if (!entry || typeof entry !== "object") return null;
    const { status, requirement_states, proven_done, ...task } = entry;
    if (!task || typeof task.task_id !== "string") return null;
    entries.set(task.task_id, canonical(normal(task)));
  }
  return entries;
}

function committed(outcome) {
  return Boolean(outcome && outcome.publicationReceipt && outcome.postcondition?.satisfied === true);
}

// Findings keyed by the frozen file name of the task each one names, in
// subject order. Unattributed findings are routed to skeleton context before
// grouping, so no open defect is discarded.
function groupFindingsBySubject(findings, subjects) {
  const grouped = new Map();
  for (const finding of findings) {
    const subject = resolveFindingSubject(finding, subjects);
    if (!subject) continue;
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
  return null;
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

// Keep unattributed defects open as skeleton context, and record their route.
let unattributedEvent = 0;
async function routeUnattributed(w, bodyFindings, skeletonFindings, subjects, round) {
  for (let i = bodyFindings.length - 1; i >= 0; i -= 1) {
    const finding = bodyFindings[i];
    if (findSubject(finding, subjects)) continue;
    bodyFindings.splice(i, 1);
    skeletonFindings.push(finding);
    if (typeof w.checkpoint === "function") {
      await w.checkpoint(`finding_unattributed-${round}-${++unattributedEvent}-${i}`, {
        task: `finding_unattributed: ${findingText(finding)}`
      });
    }
  }
}
