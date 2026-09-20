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
      judgment: { verdict: "accepted", counterexample: "", reason: "", host_call_id: "" }
    }
  ],
  supplementary: []
});

// The per-entry author gets the entries alone. Showing it the whole contract as
// the example is what made it return the whole contract (wf-379a1faa).
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

// Attempts the provider itself failed to answer. They are not the author's,
// so they get their own small budget: enough to ride out a blip, few enough
// that a dead provider stops the run promptly and says why.
const OPERATIONAL_ATTEMPTS = 3;
// A candidate the host could not even read is packaging, not authorship: the
// artifact it carried was never judged. Charging it to the candidate budget
// spent four of six acceptance attempts on quote slips inside embedded
// commands, live. Refunded, bounded, so a model that can never package one
// document still stops.
const PACKAGING_REFUNDS = 3;
// A refusal the host decided without a judge (a missing id, an unknown field,
// a floor with no verifier) names the exact defect, so repairing it is not a
// judged attempt. Refunded, bounded flat rather than per attempt: twelve
// consecutive mechanical refusals is a broken prompt, and at minutes per
// author call a larger allowance turns that into hours before the run says so.
const ACCEPTANCE_REFUSAL_REFUNDS = 12;
const PACKAGING_REFUSAL = "candidate artifact was refused: the reply is not a JSON document";
const ACCEPTANCE_ATTEMPTS = 6;
const SKELETON_ATTEMPTS = 6;
const BODY_ATTEMPTS = 10;

async function workflow(w) {
  requireFixedArgs();
  const frozen = frozenChain();

  // A stage the launcher found frozen and verified is not re-authored: the
  // host verifies it in place and that committed outcome is its evidence.
  const acceptance = frozen.acceptance ? await verifyFrozenStage(w, "verify-frozen-acceptance") : await authorCandidate(w, {
    phase: "acceptance",
    author: authorAcceptanceEntries,
    capability: "freeze-acceptance",
    attempts: ACCEPTANCE_ATTEMPTS,
    retryScopes: new Set(["candidate_artifact"]),
    prompt: () => [
      "Author exactly one acceptance entry identified below, not the whole contract.",
      `Read the PRD at ${args.prdPath}.`,
      groundingRules(),
      "Use the repository only to verify real test names and paths; every path or test the entry names must be one you observed under the repository root.",
      "Return one JSON object with id, criterion, check, gap_permitted, judgment. The two examples below are ENTRIES showing the two check shapes; your reply is one such entry and nothing around it.",
      ENTRY_SHAPES,
      "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges every entry and validates the assembled contract together.",
      "Use the exact supplied id. Criterion and judgment are host-owned placeholders. Set gap_permitted only if the PRD permits that criterion to remain a documented gap.",
      "Your entire reply must be the entry itself: the raw JSON object, starting with { and ending with }. Emit no prose, no explanation, no headings and no Markdown code fences before or after it.",
      "Do not run commands or write files."
    ].join("\n")
  });

  const skeleton = frozen.skeleton ? await verifyFrozenStage(w, "verify-frozen-skeleton") : await authorCandidate(w, {
    phase: "skeleton",
    capability: "freeze-skeleton",
    attempts: SKELETON_ATTEMPTS,
    retryScopes: new Set(["candidate_artifact", "skeleton"]),
    prompt: () => [
      "Author one complete task-skeleton JSON artifact for the frozen acceptance contract.",
      `Read the PRD at ${args.prdPath}, the task root at ${args.taskRoot}, and the repository source you need.`,
      groundingRules(),
      "Stop reading once you can name what your entries assert.",
      "Every artifact_path is a path relative to the repository root. Observe under the repository root whether each one exists before you declare it, and give every repository file or directory the PRD names by path an owning task: the host refuses a skeleton that leaves one unowned.",
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
  });
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

  const gates = await runSetGateLoop(w, skeleton.subjects, bodies);
  const evidence = [acceptance, skeleton, ...bodyEvidence(skeleton.subjects, bodies), gates.taskSetLint, gates.requirementsTrace];
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
    attempts: BODY_ATTEMPTS,
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

async function authorCandidate(w, policy) {
  let feedback = Array.isArray(policy.initialFeedback) ? policy.initialFeedback.slice() : [];
  // Seeded feedback is attempt 0 of the history: every later prompt in this
  // phase keeps showing the finding the phase was opened to repair.
  const history = feedback.length > 0 ? [{ attempt: 0, findings: feedback.slice() }] : [];
  let bestCommitted = null;
  let bestFindings = Infinity;
  let call = AUTHOR_CALLS.get(policy.phase) || 0;
  let attempt = 0;
  let operational = 0;
  let packagingRefunds = 0;
  let acceptanceRefusals = 0;
  const authorState = { entries: new Map(), retryIds: null };
  while (attempt < policy.attempts) {
    call += 1;
    AUTHOR_CALLS.set(policy.phase, call);
    const prompt = authorPrompt(policy.prompt(), attempt + 1, feedback, history);
    const authored = policy.author
      ? await policy.author(w, prompt, call, authorState)
      : await w.agent(`${policy.phase}-author-${call}`, {
          task: prompt, tier: "planner", resultMode: "rawOutcome",
          ...(policy.phase === "skeleton" ? { recordLanding: "skeleton" } : {})
        });
    // A call the host could not complete says nothing about the artifact: the
    // provider never answered. Charging it to the candidate budget spends the
    // author's attempts on an outage and then blames the author for the result.
    if (authored.status === "failed") {
      operational += 1;
      if (operational >= OPERATIONAL_ATTEMPTS) {
        throw new Error(`${policy.phase} author calls failed operationally ${operational} times: ${authored.summary || "no summary"}`);
      }
      continue;
    }
    operational = 0;
    attempt += 1;
    if (authored.stopReason !== "end_turn" || typeof authored.content !== "string" || authored.content.length === 0) {
      feedback = [`Provider outcome was incomplete (stopReason=${authored.stopReason || "missing"}); return one complete artifact.`];
      continue;
    }

    const outcome = await w.hostCommand(policy.capability, { stdin: authored.content });
    const routed = routeFindings(outcome, policy.retryScopes, policy.shadowScopes);
    if (policy.author) {
      const ids = new Set(Object.keys(args.acceptanceCriteria || {}));
      const repair = (outcome.gateEnvelope?.policy_findings || [])
        .filter(finding => policy.retryScopes.has(finding.remediation_scope));
      authorState.retryIds = acceptanceRepairIds(repair, ids, Boolean(outcome.publicationReceipt));
    }
    // A committed artifact is the best one so far, not the finished one. The
    // gate publishing in observe mode says the gate did not block; it says
    // nothing about whether the artifact still carries defects the author can
    // fix. Returning here discarded the routing computed one line above, so a
    // task set was published with 19 shadow findings — an acceptance floor the
    // gate itself reported as not falsifiable among them — and built on for a
    // week. Repairable findings are fed back below; observe still never blocks,
    // because an exhausted budget falls back to the best artifact seen.
    //
    // Best, not latest. Attempts do not improve monotonically — a live run went
    // 2 findings, 1, 2, 1, 1 and then produced a malformed candidate, so keeping
    // the most recent commit froze a worse contract than two earlier attempts
    // had already produced. Ties keep the earlier artifact: it reached this
    // quality with fewer author attempts and nothing later improved on it.
    if (outcome.publicationReceipt && outcome.postcondition?.satisfied === true) {
      if (routed.all.length < bestFindings) {
        bestCommitted = outcome;
        bestFindings = routed.all.length;
      }
    }
    if (routed.fatal.length > 0) {
      throw new Error(`${policy.phase} stopped: ${routed.fatal.join(" | ")}`);
    }
    if (routed.retry.length === 0) {
      requireCommitted(outcome, policy.phase);
      return outcome;
    }
    history.push({ attempt, findings: routed.retry.slice() });
    feedback = routed.retry;
    // Packaging keeps its own small bound (TD-027): it is a host refusal too,
    // and letting it into the mechanical allowance below would give a model
    // that never packages one document twelve calls instead of three.
    const packaging = routed.retry.every((text) => text.includes(PACKAGING_REFUSAL));
    if (packaging) {
      if (packagingRefunds < PACKAGING_REFUNDS) {
        packagingRefunds += 1;
        attempt -= 1;
      }
      continue;
    }
    const deterministicRefusal = policy.phase === "acceptance"
      && !outcome.publicationReceipt
      && routed.retry.every((text) => text.startsWith("candidate artifact was refused:"));
    if (deterministicRefusal) {
      acceptanceRefusals += 1;
      if (acceptanceRefusals >= ACCEPTANCE_REFUSAL_REFUNDS) {
        throw new Error(`acceptance exhausted ${acceptanceRefusals} deterministic repairs: ${routed.retry.join(" | ")}`);
      }
      attempt -= 1;
    }
  }

  if (args.gateMode === "observe" && bestCommitted) return bestCommitted;
  throw new Error(`${policy.phase} exhausted ${policy.attempts} candidate attempts`);
}

// The set gate's retry scopes. Every body was judged alone against its own
// claims; a finding that only the whole set reveals (an obligation task A
// claims, hollowed by text in task B) is still a body defect, and Issue-46
// sends it back to the body it names instead of ending the run.
const SET_GATE_RETRY_SCOPES = new Set(["body", "candidate_artifact"]);

async function runSetGate(w, capability) {
  const outcome = await w.hostCommand(capability, { stdin: null });
  // Set-level skeleton findings shadow-mark the run and continue.
  const routed = routeFindings(outcome, SET_GATE_RETRY_SCOPES, new Set(["skeleton"]));
  if (routed.fatal.length > 0) {
    throw new Error(`${capability} stopped: ${routed.fatal.join(" | ")}`);
  }
  return { capability, outcome, routed };
}

function acceptSetGate(gate) {
  if (args.gateMode === "enforce" && gate.routed.all.length > 0) {
    throw new Error(`${gate.capability} found enforce-policy defects: ${gate.routed.all.join(" | ")}`);
  }
  requireCommitted(gate.outcome, gate.capability);
  return gate.outcome;
}

function routeFindings(outcome, retryScopes, shadowScopes) {
  if (!outcome || typeof outcome !== "object") throw new Error("host command returned no typed outcome");
  if (outcome.gateEnvelope?.operational_error) {
    throw new Error(outcome.gateEnvelope.operational_error.text || "host gate operational failure");
  }
  const findings = Array.isArray(outcome.gateEnvelope?.policy_findings)
    ? outcome.gateEnvelope.policy_findings
    : [];
  const shadows = shadowScopes || new Set();
  const routed = { retry: [], retryFindings: [], fatal: [], inherited: [], shadow: [], all: [] };
  for (const finding of findings) {
    const text = typeof finding.text === "string" ? finding.text : "unnamed policy finding";
    const scope = finding.remediation_scope;
    routed.all.push(text);
    if (scope === "prd_input" || scope === "operational") routed.fatal.push(text);
    else if (scope === "inherited_predecessor") routed.inherited.push(text);
    else if (retryScopes.has(scope)) { routed.retry.push(text); routed.retryFindings.push(finding); }
    else if (shadows.has(scope)) routed.shadow.push(text);
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
  let prompt = `${base}\nLogical attempt: ${attempt}. Repair these exact authoritative findings:\n- ${feedback.join("\n- ")}`;
  // Two gates can be individually satisfiable and jointly hard. Without the
  // history an author repairs the finding in front of it, trips the other, and
  // alternates until its budget is spent -- a live acceptance phase did exactly
  // that for all six attempts. Showing what earlier attempts already triggered
  // is what lets it satisfy both at once instead of trading one for the other.
  const earlier = Array.isArray(history) ? history.filter((entry) => entry.findings.length > 0) : [];
  if (earlier.length > 0) {
    const lines = earlier.map((entry) => `${entry.attempt === 0 ? "the set gate, before this body was sent back" : `attempt ${entry.attempt}`}: ${entry.findings.join("; ")}`);
    prompt += `\nEarlier attempts in this phase already triggered the following. Satisfy every one of them at once; repairing the finding above by reverting an earlier repair will not converge:\n- ${lines.join("\n- ")}`;
  }
  return prompt;
}
