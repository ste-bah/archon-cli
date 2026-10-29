const ACCEPTANCE_SHAPE = JSON.stringify({
  schema_version: 1,
  prd: { path: "", digest: "" },
  gap_policy: { permitted_acceptance_ids: [], forbidden_phrases: [], required_fields: [] },
  acceptance: [
    {
      id: "<exact acceptance id defined by the PRD>",
      criterion: "",
      check: {
        kind: "floor",
        contract: {
          kind: "<deliverable kind>",
          artifact_path: "<repository-relative artifact path>",
          artifact_format: "json",
          required_true_fields: ["<field that must be true>"],
          typed_verifier_command: "<command that exercises the deliverable and fails when the criterion is false>"
        }
      },
      gap_permitted: false,
      covers: ["<requirement id defined by the PRD whose violation makes this check fail>"],
      judgment: { verdict: "accepted", counterexample: "", reason: "", host_call_id: "" }
    },
    {
      id: "<exact acceptance id defined by the PRD>",
      criterion: "",
      check: {
        kind: "command",
        // Live replies put long scripts in check blocks; inline scripts break JSON parsing.
        command_block: true,
        cwd: "project_root"
      },
      gap_permitted: false,
      covers: ["<requirement id defined by the PRD whose violation makes this check fail>"],
      judgment: { verdict: "accepted", counterexample: "", reason: "", host_call_id: "" }
    }
  ],
  supplementary: []
});

// The per-entry author gets the entries alone. Showing it the whole contract as
// the example is what made it return the whole contract (observed live).
const ENTRY_SHAPES = JSON.stringify(JSON.parse(ACCEPTANCE_SHAPE).acceptance);

const SKELETON_SHAPE = JSON.stringify({
  schema_version: 1,
  acceptance_digest: "",
  tasks: [
    {
      task_id: "<canonical task id>",
      file_name: "<canonical task id>.md",
      depends_on: [
        {
          task_id: "<canonical id of the task depended on>",
          consumes: [{ artifact_path: "<path this task reads from that one>" }],
          ordering_only: false
        }
      ],
      blocks: ["<canonical id of a task that waits on this one>"],
      implements: ["<requirement id defined by the PRD>"],
      deliverable_contracts: [
        {
          kind: "<short stable name for what this task produces>",
          artifact_path: "<path this task produces, exactly as its dependents consume it>",
          min_instances: 1
        }
      ]
    }
  ]
});

// Frozen fields are host-stamped at landing; the body author supplies the
// sections below them and does not need to reproduce the frozen values.
const BODY_SHAPE = [
  "```yaml",
  "task_id: <frozen task id>",
  "title: <short title>",
  "complexity: low|medium|high",
  "status: ready",
  "depends_on: <host stamps frozen value>",
  "blocks: <host stamps frozen value>",
  "implements: <host stamps frozen value>",
  "required_env_keys: []",
  "required_tools: <exact tools this task must exercise; [] only when no tool invocation is required>",
  "deliverable_contracts: <host stamps frozen value>",
  "```"
].join("\n");

// A candidate the host could not even read is packaging, not authorship: the
// artifact it carried was never judged. The progress rule ranks it below every
// judged or refused candidate (workflow_decompose_v1_progress.js).
const PACKAGING_REFUSAL = "candidate artifact was refused: the reply is not a JSON document";

async function workflow(w) {
  requireFixedArgs();
  if (args.phaseSeed) applySeedOrdinals(); // Issue 360: after an upgrade, start from the phase seed.
  const frozen = frozenChain();

  const { acceptance, skeleton, changedChecks } = await prepareAuthorStages(w, frozen);

  // Keyed by frozen file name so a body the set gate sends back replaces its
  // earlier evidence entry instead of appending a second one. A body already
  // on disk under the frozen chain is not authored: the set gates judge it
  // with the rest, and send it back here if they find it wanting.
  const bodies = new Map();
  const unauthored = [];
  const forceBodySubjects = new Set(Array.isArray(args.forceBodySubjects) ? args.forceBodySubjects : []);
  for (const subject of skeleton.subjects) {
    requireSubject(subject);
    if (frozenBodyReusable(frozen, skeleton, subject, changedChecks) && !forceBodySubjects.has(subject.taskId)) continue;
    unauthored.push([subject, []]);
  }
  await authorBodies(w, unauthored, bodies);

  // The skeleton the set gates stand on: re-frozen by the loop when a
  // finding only the skeleton can repair is open (Batch O).
  const chain = { outcome: skeleton, subjects: skeleton.subjects };
  const gates = await runSetGateLoop(w, chain, bodies);
  const evidence = [acceptance, chain.outcome, ...bodyEvidence(chain.subjects, bodies), gates.taskSetLint, gates.requirementsTrace];
  reconcile(evidence, bodies.size);

  return await w.finalReport("fixed-decomposition-final", {
    status: "accepted",
    inputs: evidence,
    task: "Reconcile the host-validated fixed decomposition evidence."
  });
}

function requireFixedArgs() {
  if (!args || typeof args !== "object") throw new Error("fixed decomposition args are absent");
  for (const key of ["projectRoot", "repositoryRoot", "prdPath", "prdDigest", "taskRoot"]) {
    if (typeof args[key] !== "string" || args[key].trim() === "") {
      throw new Error(`fixed decomposition argument ${key} is missing`);
    }
  }
  if (args.gateMode !== "observe" && args.gateMode !== "enforce") {
    throw new Error("fixed decomposition requires observe or enforce gate mode");
  }
}

// The skeleton's author policy, whether it authors the skeleton first or
// re-authors it because the set gate found an obligation no task claims or
// a claim the PRD does not define (Batch O). `initialFeedback` carries the
// set gate's exact findings into the first attempt.
function skeletonPolicy(initialFeedback, initialRung) {
  return {
    phase: "skeleton",
    capability: "freeze-skeleton",
    retryScopes: new Set(["candidate_artifact", "skeleton"]),
    initialFeedback,
    ...(Number.isInteger(initialRung) ? { initialRung } : {}),
    prompt: () => [
      "Author one complete task-skeleton JSON artifact for the frozen acceptance contract.",
      `Read the PRD at ${args.prdPath}, the task root at ${args.taskRoot}, and the repository source you need.`,
      groundingRules(),
      "Stop reading once you can name what your entries assert.",
      "Every artifact_path is a path relative to the repository root. Observe under the repository root whether each one exists before you declare it, and give every repository file or directory the PRD names by path an owning task: the host refuses a skeleton that leaves one unowned.",
      "Every obligation id the PRD defines is in the implements of at least one task, and implements names only ids the PRD defines. When the task root already holds a frozen skeleton, keep every task_id and file_name it freezes: repair an obligation no task claims by adding its id to the implements of the task that owns that work (or by adding a task), never by removing a task.",
      "The document must deserialize into this exact shape:",
      SKELETON_SHAPE,
      "Every <...> above is a placeholder describing the value, never a value: replace each one.",
      "One task per unit of work; task_id and file_name become the frozen tuple the bodies must preserve.",
      "Write task_id as the unquoted canonical TASK-<AREA>-<NNN>; its value must equal the TASK-<AREA>-<NNN> id in the filename, without the extension.",
      "depends_on and blocks are empty arrays when the task has no such relation; every entry present takes exactly the shape shown.",
      "Each depends_on entry declares a non-empty consumes list or ordering_only: true.",
      "The host overwrites acceptance_digest: send the placeholder shown.",
      "Generate large or repetitive fixture data with code rather than writing it literally.",
      "Your entire reply must be the artifact itself: the raw JSON document, starting with { and ending with }.",
      "Emit no prose, no explanation, no headings and no Markdown code fences before or after it.",
      "Do not run commands or write files."
    ].join("\n")
  };
}

// One body policy, whether the body is authored fresh after the skeleton or
// re-authored because the set gate named it (Issue-46). `initialFeedback`
// carries the set gate's exact finding texts into the first attempt.
function bodyPolicy(subject, initialFeedback) {
  return {
    phase: `body-${subject.taskId}`,
    capability: "land-task-body",
    target: { taskId: subject.taskId, fileName: subject.fileName },
    retryScopes: new Set(["candidate_artifact", "body"]),
    initialFeedback,
    prompt: () => [
      `Author the complete TASK body for host-frozen task_id ${subject.taskId}.`,
      `The exact frozen file_name is ${subject.fileName}.`,
      `Read the PRD at ${args.prdPath}, the frozen chain under ${args.taskRoot}, and the repository source you need.`,
      groundingRules(),
      "Every path and every file::test locator anywhere in this body is relative to the repository root, never to a crate or module directory.",
      "Stop reading once you can name what your entries assert. The project MCP configuration named below sits at the project root itself, not inside any excluded directory, and must still be read.",
      observationRule(),
      "The file must open with a fenced yaml block carrying exactly these keys:",
      BODY_SHAPE,
      "Values are yours except task_id and file_name, which must equal the frozen tuple above.",
      "Write task_id as the unquoted canonical TASK-<AREA>-<NNN>; its value must equal the TASK-<AREA>-<NNN> id in this filename, without the extension.",
      "Copy the frozen implements array exactly, preserving every value and its order.",
      `Read the project MCP configuration at ${args.projectRoot}/.mcp.json and match this task's PRD obligations to its exact permitted tool names.`,
      "Declare only task-specific invocation obligations: every declared tool must actually be called and reported in commands_run; use fully qualified mcp__server__tool names for MCP calls.",
      "An MCP deliverable cannot declare no MCP tools. List the exact MCP calls and inputs in Focused Tests; shell commands and recorded fixtures alone do not exercise MCP.",
      "Every MCP tool called by a Focused Tests command must appear in required_tools, using its exact project-permitted name from .mcp.json.",
      "HTTP/service providers are not MCP tools. Declare required environment keys only when live execution requires them; preserve explicitly permitted no-credential/unavailable paths. Never copy the ambient project toolchain into every task.",
      "For each claimed PRD obligation, decide whether it is necessarily true assuming every listed task passes its acceptance criteria and focused tests, including all allowances in its text. If it is not necessarily true, identify the weakest claiming task and quote its loophole verbatim; if it is necessarily true, leave the weakest task and quote empty.",
      "Every test file run by Focused Tests must be declared by an owning task in Files Expected to Change, a shared-append target or a deliverable contract (a declared directory may cover it); declare a test file in the Files Expected to Change of the task that owns it.",
      "Files Forbidden to Change entries must be literal paths, directories, basenames or globs, not prose descriptions.",
      "After the yaml block, use Markdown headings; include a `## Focused Tests` section whose entries are runnable commands.",
      "Preserve every frozen tuple field exactly.",
      "The yaml block is part of the file: close it with a ``` line of its own before the first Markdown heading.",
      "Generate large or repetitive fixture data with code rather than writing it literally.",
      "Your entire reply must be the TASK file itself, as raw UTF-8 Markdown.",
      "Wrap the reply in no outer code fence, and add no commentary before or after it.",
      "Do not run commands or write files."
    ].join("\n")
  };
}

// Author calls already made per phase. A body the set gate sends back keeps
// its call-id family and continues the ordinal, so on resume its earlier
// attempts replay as history and a repair never overwrites a recorded call.
const AUTHOR_CALLS = new Map();

// One subject's author loop, limited by progress (Issue 261): an attempt that
// makes progress keeps it going; STALL_ATTEMPTS in a row without progress
// advances the authored repair rung. Rung progress follows replayed calls.
async function authorCandidate(w, policy) {
  let feedback = Array.isArray(policy.initialFeedback) ? policy.initialFeedback.slice() : [];
  // Seeded feedback is attempt 0 of the history: every later prompt in this
  // phase keeps showing the finding the phase was opened to repair.
  const history = feedback.length > 0 ? [{ attempt: 0, findings: feedback.slice() }] : [];
  let call = AUTHOR_CALLS.get(policy.phase) || 0;
  let attempt = 0;
  let lastFindings = feedback.slice();
  const allFindings = feedback.slice();
  let judgeFeedback = [];
  let repairRung = Number.isInteger(policy.initialRung) ? policy.initialRung : 1;
  let repairCycle = 1;
  const progress = newProgress(feedback);
  const authorState = { entries: new Map(), retryIds: null, retryPreparation: true };
  // Issue 360: after an upgrade, start from the latest durable candidate (workflow_decompose_v1_seed.js).
  const seeded = args.phaseSeed && !seedWouldReplaceLiveFeedback(policy)
    ? seedAuthorLoop(policy, authorState)
    : null;
  if (seeded) [feedback, lastFindings] = [seeded.feedback.slice(), seeded.feedback.slice()];
  let carried = seeded ? seeded.carried : null;
  for (;;) {
    const stall = stallReason(progress);
    if (stall) {
      if (stall === "no_progress" && policy.deferAuthoredPending) {
        // Yield the stalled subject so independent bodies settle before the
        // pool pauses. A resume advances its ladder with a fresh window.
        return { pendingAuthor: true, pendingRung: repairRung, findings: lastFindings.slice() };
      }
      // A host fault is not an authored finding and cannot be repaired here.
      await pauseAuthorLoop(w, policy.phase, progress, stall, lastFindings);
      continue;
    }
    if (progress.history.length > 0 && progress.history.at(-1).kind !== "operational") {
      // Deferred bodies spend the existing window on one complete strategy cycle.
      // The first observation is the baseline; the next attempts visit rungs 2–4.
      // A rung change consumes an attempt; it never opens a fresh window.
      repairRung = repairRung === 4 ? 2 : repairRung + 1;
      if (repairRung === 2) {
        repairCycle += 1;
        // Entry authors keep their own refusal in authorState; do not copy
        // a sibling's finding into another entry's fresh-context prompt.
        feedback = policy.author ? judgeFeedback.slice()
          : [...new Set([...allFindings, ...lastFindings])];
        history.length = 0;
      }
      await repairEvent(w, policy.phase, repairRung, repairCycle, call, progress.calls);
    }
    const carry = carried;
    carried = null;
    const nextCall = carry === null ? call + 1 : call;
    const addedBefore = authorState.added || 0;
    const rungInstruction = repairRung === 1 ? ""
      : repairRung === 2 ? "\nRe-author a complete replacement from fresh context. Keep the subject contract and repair every current finding."
      : repairRung === 3 ? "\nReturn a complete replacement. Work through the findings in smaller groups while preserving every finding from this and earlier attempts."
      : "\nThis subject is deferred while independent subjects settle. Return a complete replacement when re-entered with the latest findings.";
    const midpoint = Math.ceil(feedback.length / 2);
    const groupFeedback = repairRung === 3 && feedback.length > 1
      ? (nextCall % 2 === 1 ? feedback.slice(0, midpoint) : feedback.slice(midpoint))
      : feedback;
    const promptFeedback = [...new Set([...(policy.initialFeedback || []), ...(policy.author ? feedback : [...allFindings, ...feedback])])];
    let authored;
    try {
      const operation = `\nRepair operation r${repairRung}, cycle ${repairCycle}, authored call ${nextCall}.`;
      const prompt = authorPrompt(`${policy.prompt()}${rungInstruction}${operation}${repairRung === 3 ? `\nPrioritize this group first:\n- ${groupFeedback.join("\n- ")}` : ""}`, attempt + 1, promptFeedback, history);
      if (carry !== null) authored = { status: "accepted", stopReason: "end_turn", content: carry };
      else {
        // Context writes are not journaled. Reserve no ordinal until the
        // complete prompt is ready, so recovery and replay dispatch identically.
        const task = policy.author ? prompt : await requireDispatchable(w, `${policy.phase}-prompt`, prompt);
        if (policy.author) {
          authored = await policy.author(w, task, nextCall, authorState);
          if (authorState.roundCalls > 0 || authored.status !== "failed") {
            call = nextCall;
            AUTHOR_CALLS.set(policy.phase, call);
          }
        } else {
          progress.calls += 1;
          call = nextCall;
          AUTHOR_CALLS.set(policy.phase, call);
          authored = await w.agent(`${policy.phase}-author-${call}`, {
            task, tier: "planner", resultMode: "rawOutcome",
            ...(policy.phase === "skeleton" ? { recordLanding: "skeleton" } : {})
          });
        }
      }
    } catch (error) {
      if (!error || error.authorContext !== true) throw error;
      authorState.roundCalls = 0;
      authorState.roundAnswered = 0;
      authorState.roundPassed = [];
      authorState.roundOutage = true;
      authored = { status: "failed", summary: error.message };
    }
    const measuredReplies = policy.author && authorState.roundCalls !== undefined;
    if (measuredReplies) {
      progress.calls += authorState.roundCalls;
      progress.answered += authorState.roundAnswered;
    }
    // A round without a candidate can still retain work when a sibling fails:
    // a previously missing entry lowers the outstanding-entry count. A
    // rewrite alone clears no defect; every failure shares the same window.
    const advanced = (authorState.added || 0) > addedBefore;
    const round = authorRoundReport(policy, authorState);
    if (authored.status === "failed") {
      // Every kind of failed round credits the entries that passed in it and
      // counts any outage in it. Each entry reads its own refusal or note;
      // the shared feedback stays the gate's findings.
      if (authored.refusals) {
        recordRepairs(progress, call, authored.refusals, round, !measuredReplies, advanced);
        lastFindings = authored.findings.map(progressText);
      } else if (authored.malformed) {
        recordAnswered(progress, call, "entries", advanced, !measuredReplies, round);
        lastFindings = [authored.summary || "malformed replies"];
      } else {
        recordOperational(progress, call, authored.summary, advanced, round, "provider");
        lastFindings = [`author call failed operationally: ${authored.summary || "no summary"}`];
      }
      if (authored.refusals || authored.malformed) {
        attempt += 1;
        for (const finding of lastFindings) if (!allFindings.includes(finding)) allFindings.push(finding);
      }
      continue;
    }
    if (authored.stopReason !== "end_turn" || typeof authored.content !== "string" || authored.content.length === 0) {
      attempt += 1;
      feedback = [authored.stopReason === "max_tokens"
        ? incompleteReplyFeedback(authored.content)
        : `Provider outcome was incomplete (stopReason=${authored.stopReason || "missing"}); return one complete artifact.`];
      lastFindings = feedback.slice();
      for (const finding of feedback) if (!allFindings.includes(finding)) allFindings.push(finding);
      recordAnswered(progress, call, "incomplete", false, !measuredReplies);
      continue;
    }

    const outcome = await w.hostCommand(policy.capability, {
      stdin: authored.content,
      ...(policy.target ? { target: policy.target } : {})
    });
    if (policy.author && outcome && outcome.publicationReceipt) clearPublishedDrafts(authorState);
    const routed = routeFindings(outcome, policy.retryScopes, policy.shadowScopes);
    if (routed.operational) {
      if (routed.operationalKind === "authored_judge_reply") {
        // The judge could not use this candidate. Feed its reason to the
        // author so the next call creates a replacement.
        if (policy.author) {
          const named = /still have no [^(]*\(([^)]*)\)/.exec(routed.operational);
          const namedIds = new Set(named ? named[1].split(",").map(id => id.trim()) : []);
          const retryIds = (authorState.order || []).filter(id => namedIds.has(id));
          authorState.retryIds = retryIds.length > 0 ? new Set(retryIds) : null;
        }
        recordAnswered(progress, call, "authored_judge_reply", advanced, !measuredReplies, round);
        feedback = [`freeze judge reply unusable: ${routed.operational}`];
        judgeFeedback = feedback.slice();
        lastFindings = feedback.slice();
        for (const finding of feedback) if (!allFindings.includes(finding)) allFindings.push(finding);
        continue;
      }
      // The gate never judged the candidate; only the round that made it can
      // be progress: a pass in it, or a previously missing entry it added.
      if (!measuredReplies) progress.answered += 1;
      recordOperational(progress, call, routed.operational, advanced, round, policy.capability);
      lastFindings = [`${policy.capability}: ${routed.operational}`];
      carried = authored.content;
      continue;
    }
    const ownedFatal = routed.fatalFindings.filter((finding) => {
      if (finding.remediation_scope === "operational") return false;
      const source = typeof finding.source_path === "string" ? finding.source_path.replace(/\\/g, "/") : "";
      if (policy.target && authoredSourceMatches(source, policy.target.fileName)) return true;
      return policy.phase === "acceptance" && (source === `${String(args.taskRoot || "").replace(/\\/g, "/")}/acceptance-contract.json` || source === "acceptance-contract.json")
        && typeof finding.subject === "string"
        && (Object.prototype.hasOwnProperty.call(args.acceptanceCriteria || {}, finding.subject)
          || authorState.entries.has(finding.subject));
    });
    if (ownedFatal.length > 0) {
      const owned = new Set(ownedFatal);
      const ownedText = ownedFatal.map((finding) => String(finding.text || "unnamed policy finding"));
      routed.fatal = routed.fatal.filter((_, index) => !owned.has(routed.fatalFindings[index]));
      routed.fatalOperational = routed.fatalOperational.filter((_, index) => !owned.has(routed.fatalFindings[index]));
      routed.fatalFindings = routed.fatalFindings.filter((finding) => !owned.has(finding));
      routed.retry.push(...ownedText);
      routed.retryFindings.push(...ownedFatal);
    }
    if (policy.author) {
      const ids = new Set(Object.keys(args.acceptanceCriteria || {}));
      const repair = routed.retryFindings;
      if (!authorState.gateFindings) authorState.gateFindings = new Map();
      for (const finding of repair) {
        const selected = ids.has(finding.subject) ? new Set([finding.subject])
          : acceptanceRepairIds([finding], ids, Boolean(outcome.publicationReceipt), JSON.parse(authored.content), authorState.entries);
        for (const id of selected || authorState.entries.keys()) {
          const prior = authorState.gateFindings.get(id) || [];
          if (!prior.includes(finding.text)) prior.push(finding.text);
          authorState.gateFindings.set(id, prior);
        }
      }
      const stale = repair.find(finding => finding?.deterministic_defect?.code === "stale_acceptance_contract");
      if (stale) {
        // The contract changed after this freeze was prepared: rebuild the
        // candidate from the current published contract the host returned.
        // With nothing else to repair the rebuilt candidate is frozen again at
        // once, inside the ordinary no-progress window.
        const rebuilt = refreshFromCurrentContract(authorState, outcome.gateEnvelope.report,
          repair.filter(finding => finding !== stale), ids, JSON.parse(authored.content));
        if (rebuilt !== null) {
          recordAnswered(progress, call, "stale_acceptance_contract", advanced, !measuredReplies, round);
          lastFindings = [String(stale.text)];
          carried = rebuilt;
          continue;
        }
      } else {
        const selected = repair.map(finding => ownedFatal.includes(finding)
          ? {...finding, text: `check '${finding.subject}': ${finding.text}`} : finding);
        authorState.retryIds = acceptanceRepairIds(selected, ids, Boolean(outcome.publicationReceipt), JSON.parse(authored.content), authorState.entries);
      }
    }
    // A committed artifact is not the finished one: repairable findings are
    // still fed back below in either mode, and a loop that stalls on them
    // re-authors in either mode. A check the host could not prove
    // (an operational finding) beside findings the author can repair stops
    // nothing: it blocks publication, the author repairs its own, and the
    // next freeze probes the unproven check again.
    const fatal = routed.retry.length > 0
      ? routed.fatal.filter((text, index) => !routed.fatalOperational[index])
      : routed.fatal;
    if (fatal.length > 0) {
      await stopFixed(`${policy.phase} stopped: ${fatal.join(" | ")}`);
    }
    if (routed.retry.length === 0) {
      requireCommitted(outcome, policy.phase);
      if (policy.phase === "skeleton") recordSkeletonTasks(outcome, authored.content);
      return outcome;
    }
    attempt += 1;
    history.push({ attempt, findings: routed.retry.slice() });
    feedback = routed.retry;
    if (policy.author) judgeFeedback = feedback.slice();
    lastFindings = routed.retry.slice();
    for (const finding of feedback) if (!allFindings.includes(finding)) allFindings.push(finding);
    // `advanced` is not credited here on purpose: the gate measured the whole
    // candidate, and that measure decides (Issue 261).
    recordAttempt(progress, call, routed.retryFindings, !measuredReplies, round);
    openRepairEpisode(progress);
  }
}
