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
// opening feedback, and asks both gates again. Bounded: a set that cannot be
// made whole in SET_GATE_ROUNDS stops with the findings listed.
const SET_GATE_ROUNDS = 4;

async function runSetGateLoop(w, subjects, bodies) {
  let last = null;
  for (let round = 1; round <= SET_GATE_ROUNDS; round += 1) {
    const taskSetLint = await runSetGate(w, "task-set-lint");
    const requirementsTrace = await runSetGate(w, "requirements-trace");
    last = { taskSetLint, requirementsTrace };
    const retry = [...taskSetLint.routed.retryFindings, ...requirementsTrace.routed.retryFindings];
    if (retry.length === 0) {
      return {
        taskSetLint: acceptSetGate(taskSetLint),
        requirementsTrace: acceptSetGate(requirementsTrace)
      };
    }
    for (const [fileName, findings] of groupFindingsBySubject(retry, subjects)) {
      const subject = subjects.find((candidate) => candidate.fileName === fileName);
      const texts = findings.map((finding) => findingText(finding));
      bodies.set(fileName, await authorCandidate(w, bodyPolicy(subject, texts)));
    }
  }
  const open = [...last.taskSetLint.routed.retry, ...last.requirementsTrace.routed.retry];
  // Observe never blocks: an exhausted loop falls back to the last committed
  // gate outcomes, exactly as authorCandidate falls back to its best commit.
  if (args.gateMode === "observe" && committed(last.taskSetLint.outcome) && committed(last.requirementsTrace.outcome)) {
    return { taskSetLint: last.taskSetLint.outcome, requirementsTrace: last.requirementsTrace.outcome };
  }
  throw new Error(`set gates exhausted ${SET_GATE_ROUNDS} repair rounds with findings still open: ${open.join(" | ")}`);
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
  throw new Error(`set gate finding names no frozen task (source_path=${path || "none"}, subject=${finding?.subject ?? "none"}): ${text}`);
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
