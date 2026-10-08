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
        command: "<shell command that exercises the deliverable and exits non-zero when the criterion is false>",
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

// Frozen fields are copied, not re-decided. Showing them empty taught bodies to
// emit `[]` against a skeleton that declared real values, which the frozen-field
// comparison then reports as the body having changed them.
const BODY_SHAPE = [
  "```yaml",
  "task_id: <frozen task id>",
  "title: <short title>",
  "complexity: low|medium|high",
  "status: ready",
  "depends_on: <copy this task's frozen depends_on exactly>",
  "blocks: <copy this task's frozen blocks exactly>",
  "implements: <copy this task's frozen implements exactly>",
  "required_env_keys: []",
  "required_tools: <exact tools this task must exercise; [] only when no tool invocation is required>",
  "deliverable_contracts: <copy this task's frozen deliverable_contracts exactly>",
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

  // A stage the launcher found frozen and verified is not re-authored: the
  // host verifies it in place and that committed outcome is its evidence.
  const acceptance = frozen.acceptance ? await verifyFrozenStage(w, "verify-frozen-acceptance") : await authorCandidate(w, {
    phase: "acceptance",
    author: authorAcceptanceEntries,
    capability: "freeze-acceptance",
    retryScopes: new Set(["candidate_artifact"]),
    prompt: acceptanceAuthorPrompt
  });

  const skeleton = frozen.skeleton ? await verifyFrozenStage(w, "verify-frozen-skeleton") : await authorCandidate(w, skeletonPolicy([]));
  if (!Array.isArray(skeleton.subjects) || skeleton.subjects.length === 0) {
    throw new Error("frozen skeleton returned zero host-read task subjects");
  }
  requireFrozenSubjects(frozen, skeleton);

  // Keyed by frozen file name so a body the set gate sends back replaces its
  // earlier evidence entry instead of appending a second one. A body already
  // on disk under the frozen chain is not authored: the set gates judge it
  // with the rest, and send it back here if they find it wanting.
  const bodies = new Map();
  const unauthored = [];
  for (const subject of skeleton.subjects) {
    requireSubject(subject);
    if (frozen.bodies.has(subject.fileName)) continue;
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
function skeletonPolicy(initialFeedback) {
  return {
    phase: "skeleton",
    capability: "freeze-skeleton",
    retryScopes: new Set(["candidate_artifact", "skeleton"]),
    initialFeedback,
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
      "depends_on and blocks are empty arrays when the task has no such relation; every entry present takes exactly the shape shown.",
      "Each depends_on entry declares a non-empty consumes list or ordering_only: true.",
      "The host overwrites acceptance_digest: send the placeholder shown.",
      "Your entire reply must be the artifact itself: the raw JSON document, starting with { and ending with }.",
      "Emit no prose, no explanation, no headings and no Markdown code fences before or after it.",
      "Do not run commands or write files."
    ].join("\n")
  };
}

// Where the authors read code (Issue-55). The repository root is the ONLY
// place source paths, test names, module layout and "exists / does not exist"
// claims are verified; projectRoot holds the PRD, the task root and .mcp.json
// and nothing an author may cite as source. Before this the authors were told
// to read "repository source under the project root", which was the working
// directory: where that is not the code repository they globbed an empty
// tree (105 live runs read the repository 0-3 times each) and wrote tasks
// claiming that files which exist "do not exist".
function groundingRules() {
  return [
    `The code repository is ${args.repositoryRoot}. It is the ONLY place to verify source paths, test names, module layout and whether a file exists or does not exist: a repository path you name must be one you observed there. Never descend into any directory named: ${excludedDirs()}. Those hold dependencies, build output and earlier runs' evidence, and are the overwhelming majority of files under that root.`,
    `${args.projectRoot} is the project root. It holds the PRD, the task root and .mcp.json, and that is all you read from it: source is read under the repository root alone, even when the two directories coincide, and a file absent from the project root is not thereby absent from the repository.`
  ].join("\n");
}

// The observation every deliverable path must carry (Issue-56). The host lint
// (`topology_lint/repository_observations.rs`) parses exactly this grammar and
// checks it against the checkout, line count included: a path with no
// observation, an inexact count, or "not observed" wording is a blocking
// finding, so the words here and the parser there must agree.
function observationRule() {
  return [
    `Every path you list under Files Expected to Change, every deliverable_contracts artifact_path and every shared_append_target_files entry is a deliverable path. For each one, read it under the repository root and write, directly after the backticked path (\`${args.repositoryRoot}/<relative path>\` or the repository-relative path), exactly one of these three observations and nothing in their place:`,
    "  \`<path>\` — exists (N lines)   for a file, where N is the last line number the Read tool shows once you have read the whole file (read again with an offset when the first read stops before the end);",
    "  \`<path>\` — exists (directory)   for a directory;",
    "  \`<path>\` — absent   for a path that is not in the repository.",
    "The host checks each observation against the repository at its recorded base commit and in the checkout, the line count exactly. A deliverable path with no such observation, an approximate or placeholder count, or words such as \"not observed\", \"could not read\" or \"outside allowed\" is a blocking finding that sends the body back to you: never defer an observation to the implementer. Never write that any repository path exists or does not exist unless you observed it under the repository root."
  ].join("\n");
}

// Directory names the host says are not worth reading, joined for a prompt.
// Sourced from args so the engine's canonical list stays the single definition.
function excludedDirs() {
  const list = Array.isArray(args.excludedDirs) ? args.excludedDirs : [];
  return list.length > 0 ? list.join(', ') : '.git, node_modules, target';
}

// One body policy, whether the body is authored fresh after the skeleton or
// re-authored because the set gate named it (Issue-46). `initialFeedback`
// carries the set gate's exact finding texts into the first attempt.
function bodyPolicy(subject, initialFeedback) {
  return {
    phase: `body-${subject.taskId}`,
    capability: "land-task-body",
    retryScopes: new Set(["candidate_artifact", "body"]),
    initialFeedback,
    prompt: () => [
      `Author the complete TASK body for host-frozen task_id ${subject.taskId}.`,
      `The exact frozen file_name is ${subject.fileName}.`,
      `Read the PRD at ${args.prdPath}, the frozen chain under ${args.taskRoot}, and the repository source you need.`,
      groundingRules(),
      "Stop reading once you can name what your entries assert. The project MCP configuration named below sits at the project root itself, not inside any excluded directory, and must still be read.",
      observationRule(),
      "The file must open with a fenced yaml block carrying exactly these keys:",
      BODY_SHAPE,
      "Values are yours except task_id and file_name, which must equal the frozen tuple above.",
      `Read the project MCP configuration at ${args.projectRoot}/.mcp.json and match this task's PRD obligations to its exact permitted tool names.`,
      "Declare only task-specific invocation obligations: every declared tool must actually be called and reported in commands_run; use fully qualified mcp__server__tool names for MCP calls.",
      "An MCP deliverable cannot declare no MCP tools. List the exact MCP calls and inputs in Focused Tests; shell commands and recorded fixtures alone do not exercise MCP.",
      "HTTP/service providers are not MCP tools. Declare required environment keys only when live execution requires them; preserve explicitly permitted no-credential/unavailable paths. Never copy the ambient project toolchain into every task.",
      "Write implements as the single-line flow sequence shown; a block list leaves the file unreadable to the requirements trace.",
      "After the yaml block, use Markdown headings; include a `## Focused Tests` section whose entries are runnable commands.",
      "Preserve every frozen tuple field exactly.",
      "The yaml block is part of the file: close it with a ``` line of its own before the first Markdown heading.",
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
// pause the run with evidence, and a resumed run continues from the pause
// with a fresh window.
async function authorCandidate(w, policy) {
  let feedback = Array.isArray(policy.initialFeedback) ? policy.initialFeedback.slice() : [];
  // Seeded feedback is attempt 0 of the history: every later prompt in this
  // phase keeps showing the finding the phase was opened to repair.
  const history = feedback.length > 0 ? [{ attempt: 0, findings: feedback.slice() }] : [];
  let lastCommitted = null;
  let call = AUTHOR_CALLS.get(policy.phase) || 0;
  let attempt = 0;
  let lastFindings = feedback.slice();
  const progress = newProgress(feedback);
  const authorState = { entries: new Map(), retryIds: null };
  // Issue 360: after an upgrade, start from the latest durable candidate (workflow_decompose_v1_seed.js).
  const seeded = args.phaseSeed ? seedAuthorLoop(policy, authorState) : null;
  if (seeded) [feedback, lastFindings] = [seeded.feedback.slice(), seeded.feedback.slice()];
  let carried = seeded ? seeded.carried : null;
  for (;;) {
    const stall = stallReason(progress);
    if (stall) {
      // Observe never blocks on the artifact's quality: a loop that stopped
      // improving returns the artifact the tree holds now. An outage says
      // nothing about the artifact, so a window that holds one pauses in
      // either mode (resumable), whatever the rest of the window was.
      if (args.gateMode === "observe" && lastCommitted && !windowHasOutage(progress)) return lastCommitted;
      await pauseAuthorLoop(w, policy.phase, progress, stall, lastFindings);
    }
    const carry = carried;
    carried = null;
    if (carry === null) { call += 1; progress.calls += 1; AUTHOR_CALLS.set(policy.phase, call); }
    const addedBefore = authorState.added || 0;
    const prompt = authorPrompt(policy.prompt(), attempt + 1, feedback, history);
    const authored = carry !== null ? { status: "accepted", stopReason: "end_turn", content: carry } : policy.author
      ? await policy.author(w, prompt, call, authorState)
      : await w.agent(`${policy.phase}-author-${call}`, {
          task: prompt, tier: "planner", resultMode: "rawOutcome",
          ...(policy.phase === "skeleton" ? { recordLanding: "skeleton" } : {})
        });
    const measuredReplies = policy.author && authorState.roundCalls !== undefined;
    if (measuredReplies) {
      progress.calls += authorState.roundCalls - 1;
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
        recordOperational(progress, call, authored.summary, advanced, round);
        lastFindings = [`author call failed operationally: ${authored.summary || "no summary"}`];
      }
      continue;
    }
    attempt += 1;
    if (authored.stopReason !== "end_turn" || typeof authored.content !== "string" || authored.content.length === 0) {
      feedback = [`Provider outcome was incomplete (stopReason=${authored.stopReason || "missing"}); return one complete artifact.`];
      lastFindings = feedback.slice();
      recordAnswered(progress, call, "incomplete", false, !measuredReplies);
      continue;
    }

    const outcome = await w.hostCommand(policy.capability, { stdin: authored.content });
    const routed = routeFindings(outcome, policy.retryScopes, policy.shadowScopes);
    if (routed.operational) {
      // The gate never judged the candidate; only the round that made it can
      // be progress: a pass in it, or a previously missing entry it added.
      if (!measuredReplies) progress.answered += 1;
      recordOperational(progress, call, routed.operational, advanced, round);
      lastFindings = [`host gate operational failure: ${routed.operational}`];
      continue;
    }
    if (policy.author) {
      const ids = new Set(Object.keys(args.acceptanceCriteria || {}));
      const repair = (outcome.gateEnvelope?.policy_findings || [])
        .filter(finding => policy.retryScopes.has(finding.remediation_scope));
      authorState.retryIds = acceptanceRepairIds(repair, ids, Boolean(outcome.publicationReceipt), JSON.parse(authored.content));
    }
    // A committed artifact is not the finished one: repairable findings are
    // still fed back below in either mode. Every publication replaces the
    // live tree, so observe's fallback is the LATEST committed outcome: only
    // its receipt and subjects describe what the tree now holds (an earlier
    // outcome with fewer findings would name a skeleton that is gone).
    if (outcome.publicationReceipt && outcome.postcondition?.satisfied === true) lastCommitted = outcome;
    if (routed.fatal.length > 0) {
      await stopFixed(`${policy.phase} stopped: ${routed.fatal.join(" | ")}`);
    }
    if (routed.retry.length === 0) {
      requireCommitted(outcome, policy.phase);
      return outcome;
    }
    history.push({ attempt, findings: routed.retry.slice() });
    feedback = routed.retry;
    lastFindings = routed.retry.slice();
    // `advanced` is not credited here on purpose: the gate measured the whole
    // candidate, and that measure decides (Issue 261).
    recordAttempt(progress, call, routed.retryFindings, !measuredReplies, round);
    openRepairEpisode(progress);
  }
}

// The set gate's retry scopes. Every body was judged alone against its own
// claims; a finding that only the whole set reveals (an obligation task A
// claims, hollowed by text in task B) is still a body defect, and Issue-46
// sends it back to the body it names instead of ending the run.
const SET_GATE_RETRY_SCOPES = new Set(["body", "candidate_artifact"]);

async function runSetGate(w, capability) {
  const outcome = await w.hostCommand(capability, { stdin: null });
  // Set-level skeleton findings are routed to a skeleton re-author by the
  // loop (Batch O), never shadowed past.
  const routed = routeFindings(outcome, SET_GATE_RETRY_SCOPES, new Set(["skeleton"]));
  if (routed.fatal.length > 0) {
    await stopFixed(`${capability} stopped: ${routed.fatal.join(" | ")}`);
  }
  return { capability, outcome, routed };
}

// Protocol v1 deliberately changes the embedded script digest for a planned release.
// Only an actual validated host request makes this stop terminal; error text cannot.
async function stopFixed(reason) {
  await __archonHost("terminalStop", JSON.stringify({ schemaVersion: 1, reason: String(reason).slice(0, 4096) }));
  throw new Error("host returned without honoring the terminal stop");
}

// Batch O: a set gate is accepted only with no finding open, in either
// mode. Observe used to accept one with findings and build on it.
function acceptSetGate(gate) {
  if (gate.routed.all.length > 0) {
    throw new Error(`${gate.capability} cannot be accepted with findings open: ${gate.routed.all.join(" | ")}`);
  }
  requireCommitted(gate.outcome, gate.capability);
  return gate.outcome;
}

// A gate operational error (a truncated judge reply, a preparation failure)
// is returned as `operational`, never thrown: the caller's loop counts it as
// an attempt without progress and pauses the run when its window closes.
function routeFindings(outcome, retryScopes, shadowScopes) {
  if (!outcome || typeof outcome !== "object") throw new Error("host command returned no typed outcome");
  const routed = {
    retry: [], retryFindings: [], fatal: [], inherited: [], inheritedFindings: [],
    shadow: [], shadowFindings: [], all: [], allFindings: [], operational: null
  };
  if (outcome.gateEnvelope?.operational_error) {
    routed.operational = String(outcome.gateEnvelope.operational_error.text || "host gate operational failure");
    return routed;
  }
  const findings = Array.isArray(outcome.gateEnvelope?.policy_findings)
    ? outcome.gateEnvelope.policy_findings
    : [];
  const shadows = shadowScopes || new Set();
  for (const finding of findings) {
    const text = typeof finding.text === "string" ? finding.text : "unnamed policy finding";
    const scope = finding.remediation_scope;
    routed.all.push(text);
    routed.allFindings.push(finding);
    if (scope === "prd_input" || scope === "operational") routed.fatal.push(text);
    else if (scope === "inherited_predecessor") { routed.inherited.push(text); routed.inheritedFindings.push(finding); }
    else if (retryScopes.has(scope)) { routed.retry.push(text); routed.retryFindings.push(finding); }
    else if (shadows.has(scope)) { routed.shadow.push(text); routed.shadowFindings.push(finding); }
    // Every remaining finding stops the phase. A missing or unrecognised scope
    // is operational by contract, and a scope this phase cannot act on means
    // something the candidate does not own changed underneath it. Falling
    // through here silently left retry and fatal both empty, so the phase
    // accepted the artifact and reported success.
    else routed.fatal.push(`scope '${scope || "missing"}' is not actionable in this phase: ${text}`);
  }
  return routed;
}

function requireCommitted(outcome, phase) {
  if (!outcome.publicationReceipt) throw new Error(`${phase} returned no committed publication receipt`);
  if (!outcome.postcondition || outcome.postcondition.satisfied !== true) {
    throw new Error(`${phase} authoritative postcondition is not satisfied`);
  }
}

// `authored` is the number of body outcomes this run carries: acceptance,
// skeleton, one per subject this run authored or re-authored, and the two set
// gates. A body the frozen chain supplied and the set gates never sent back
// has no author outcome; the two committed set-gate outcomes, whose
// postcondition compares every task on disk to the frozen skeleton, are its
// evidence.
function reconcile(evidence, authored) {
  if (!Array.isArray(evidence) || evidence.length !== authored + 4) {
    throw new Error("Phase E evidence cardinality does not match frozen subjects");
  }
  for (const [index, outcome] of evidence.entries()) {
    requireCommitted(outcome, `Phase E evidence ${index + 1}`);
    const receiptId = outcome.publicationReceipt.call_id;
    const recordedId = outcome.result?.data?.publicationReceipt?.call_id;
    if (typeof receiptId !== "string" || receiptId.length === 0 || receiptId !== recordedId) {
      throw new Error(`Phase E receipt identity mismatch at evidence ${index + 1}`);
    }
  }
}

function requireSubject(subject) {
  if (!subject || typeof subject.taskId !== "string" || typeof subject.fileName !== "string") {
    throw new Error("frozen skeleton returned a malformed host-read subject");
  }
}

function authorPrompt(base, attempt, feedback, history) {
  if (feedback.length === 0) return `${base}\nLogical attempt: ${attempt}.`;
  const earlier = earlierFindings(history, feedback);
  const repeat = (text) => {
    const seen = earlier.repeats.get(historyKey(text));
    return seen ? ` (a repeat: seen in ${seen.count} earlier attempt${seen.count === 1 ? "" : "s"}, ${seen.first === seen.last ? `attempt ${seen.last}` : `attempts ${seen.first}-${seen.last}`})` : "";
  };
  let prompt = `${base}\nLogical attempt: ${attempt}. Repair these exact authoritative findings:\n- ${feedback.map((text) => `${text}${repeat(text)}`).join("\n- ")}`;
  // Two gates can be individually satisfiable and jointly hard. Without the
  // history an author repairs the finding in front of it, trips the other, and
  // alternates until its budget is spent -- a live acceptance phase did exactly
  // that for all six attempts. Showing what earlier attempts already triggered
  // is what lets it satisfy both at once instead of trading one for the other.
  // Issue 288: each earlier finding once, bounded (earlierFindings).
  if (earlier.lines.length > 0) {
    prompt += `\nEarlier attempts in this phase already triggered the following. Satisfy every one of them at once; repairing the finding above by reverting an earlier repair will not converge:\n- ${earlier.lines.join("\n- ")}`;
  }
  if (earlier.omitted > 0) {
    prompt += `\n${earlier.omitted} older distinct findings (${earlier.omittedOccurrences} occurrences, attempts ${earlier.omittedRange}) are not repeated here.`;
  }
  return prompt;
}
