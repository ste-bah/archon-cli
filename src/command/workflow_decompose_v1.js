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
          required_true_fields: ["<field that must be true>"]
        }
      },
      gap_permitted: false,
      judgment: { verdict: "accepted", counterexample: "", reason: "", host_call_id: "" }
    }
  ],
  supplementary: []
});

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
      deliverable_contracts: []
    }
  ]
});

const BODY_SHAPE = [
  "```yaml",
  "task_id: <frozen task id>",
  "title: <short title>",
  "complexity: low|medium|high",
  "status: ready",
  "depends_on: []",
  "blocks: []",
  "implements: [<requirement id defined by the PRD>]",
  "required_env_keys: []",
  "required_tools: []",
  "deliverable_contracts: []",
  "```"
].join("\n");

const ACCEPTANCE_ATTEMPTS = 6;
const SKELETON_ATTEMPTS = 6;
const BODY_ATTEMPTS = 10;

async function workflow(w) {
  requireFixedArgs();

  const acceptance = await authorCandidate(w, {
    phase: "acceptance",
    capability: "freeze-acceptance",
    attempts: ACCEPTANCE_ATTEMPTS,
    retryScopes: new Set(["candidate_artifact"]),
    prompt: () => [
      "Author one complete acceptance-contract JSON artifact.",
      `Read the PRD at ${args.prdPath} and relevant repository files under ${args.projectRoot}.`,
      "The document must deserialize into this exact shape:",
      ACCEPTANCE_SHAPE,
      "Every <...> above is a placeholder describing the value, never a value: replace each one.",
      "One acceptance entry per acceptance obligation the PRD defines, keyed by its exact id from the PRD.",
      "The host overwrites prd, gap_policy, criterion text and every judgment: send the placeholders shown.",
      "Your entire reply must be the artifact itself: the raw JSON document, starting with { and ending with }.",
      "Emit no prose, no explanation, no headings and no Markdown code fences before or after it.",
      "Do not run commands or write files."
    ].join("\n")
  });

  const skeleton = await authorCandidate(w, {
    phase: "skeleton",
    capability: "freeze-skeleton",
    attempts: SKELETON_ATTEMPTS,
    retryScopes: new Set(["candidate_artifact", "skeleton"]),
    prompt: () => [
      "Author one complete task-skeleton JSON artifact for the frozen acceptance contract.",
      `Read the PRD at ${args.prdPath}, the task root at ${args.taskRoot}, and relevant repository files.`,
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

  const bodies = [];
  for (const subject of skeleton.subjects) {
    requireSubject(subject);
    bodies.push(await authorCandidate(w, {
      phase: `body-${subject.taskId}`,
      capability: "land-task-body",
      attempts: BODY_ATTEMPTS,
      retryScopes: new Set(["candidate_artifact", "body"]),
      prompt: () => [
        `Author the complete TASK body for host-frozen task_id ${subject.taskId}.`,
        `The exact frozen file_name is ${subject.fileName}.`,
        `Read the PRD at ${args.prdPath}, the frozen chain under ${args.taskRoot}, and relevant repository files.`,
        "The file must open with a fenced yaml block carrying exactly these keys:",
        BODY_SHAPE,
        "Values are yours except task_id and file_name, which must equal the frozen tuple above.",
        "After the yaml block, use Markdown headings; include a `## Focused Tests` section whose entries are runnable commands.",
        "Preserve every frozen tuple field exactly.",
        "The yaml block is part of the file: close it with a ``` line of its own before the first Markdown heading.",
        "Your entire reply must be the TASK file itself, as raw UTF-8 Markdown.",
        "Wrap the reply in no outer code fence, and add no commentary before or after it.",
        "Do not run commands or write files."
      ].join("\n")
    }));
  }

  const taskSetLint = await runSetGate(w, "task-set-lint");
  const requirementsTrace = await runSetGate(w, "requirements-trace");
  const evidence = [acceptance, skeleton, ...bodies, taskSetLint, requirementsTrace];
  reconcile(evidence, skeleton.subjects);

  return await w.finalReport("fixed-decomposition-final", {
    status: "accepted",
    inputs: evidence,
    task: "Reconcile the host-validated fixed decomposition evidence."
  });
}

function requireFixedArgs() {
  if (!args || typeof args !== "object") throw new Error("fixed decomposition args are absent");
  for (const key of ["projectRoot", "prdPath", "prdDigest", "taskRoot"]) {
    if (typeof args[key] !== "string" || args[key].trim() === "") {
      throw new Error(`fixed decomposition argument ${key} is missing`);
    }
  }
  if (args.gateMode !== "observe" && args.gateMode !== "enforce") {
    throw new Error("fixed decomposition requires observe or enforce gate mode");
  }
}

async function authorCandidate(w, policy) {
  let feedback = [];
  let lastCommitted = null;
  for (let attempt = 1; attempt <= policy.attempts; attempt += 1) {
    const authored = await w.agent(`${policy.phase}-author-${attempt}`, {
      task: authorPrompt(policy.prompt(), attempt, feedback),
      tier: "planner",
      resultMode: "rawOutcome"
    });
    if (authored.stopReason !== "end_turn" || typeof authored.content !== "string" || authored.content.length === 0) {
      feedback = [`Provider outcome was incomplete (stopReason=${authored.stopReason || "missing"}); return one complete artifact.`];
      continue;
    }

    const outcome = await w.hostCommand(policy.capability, { stdin: authored.content });
    const routed = routeFindings(outcome, policy.retryScopes);
    if (outcome.publicationReceipt && outcome.postcondition?.satisfied === true) {
      lastCommitted = outcome;
      // Observe mode shadows: the host already committed this phase, so its
      // findings are evidence, not a verdict. Re-authoring here would spend the
      // whole attempt budget re-deciding something the gate has published, and
      // some findings — a PRD that mandates a commandless floor, say — are not
      // the author's to repair at all.
      if (args.gateMode === "observe") {
        return outcome;
      }
    }
    if (routed.fatal.length > 0) {
      throw new Error(`${policy.phase} stopped: ${routed.fatal.join(" | ")}`);
    }
    if (routed.retry.length === 0) {
      requireCommitted(outcome, policy.phase);
      return outcome;
    }
    feedback = routed.retry;
  }

  if (args.gateMode === "observe" && lastCommitted) return lastCommitted;
  throw new Error(`${policy.phase} exhausted ${policy.attempts} candidate attempts`);
}

async function runSetGate(w, capability) {
  const outcome = await w.hostCommand(capability, { stdin: null });
  const routed = routeFindings(outcome, new Set());
  if (routed.fatal.length > 0) {
    throw new Error(`${capability} stopped: ${routed.fatal.join(" | ")}`);
  }
  if (args.gateMode === "enforce" && routed.all.length > 0) {
    throw new Error(`${capability} found enforce-policy defects: ${routed.all.join(" | ")}`);
  }
  requireCommitted(outcome, capability);
  return outcome;
}

function routeFindings(outcome, retryScopes) {
  if (!outcome || typeof outcome !== "object") throw new Error("host command returned no typed outcome");
  if (outcome.gateEnvelope?.operational_error) {
    throw new Error(outcome.gateEnvelope.operational_error.text || "host gate operational failure");
  }
  const findings = Array.isArray(outcome.gateEnvelope?.policy_findings)
    ? outcome.gateEnvelope.policy_findings
    : [];
  const routed = { retry: [], fatal: [], inherited: [], all: [] };
  for (const finding of findings) {
    const text = typeof finding.text === "string" ? finding.text : "unnamed policy finding";
    const scope = finding.remediation_scope;
    routed.all.push(text);
    if (scope === "prd_input" || scope === "operational") routed.fatal.push(text);
    else if (scope === "inherited_predecessor") routed.inherited.push(text);
    else if (retryScopes.has(scope)) routed.retry.push(text);
    else if (scope === "body") routed.fatal.push(text);
  }
  return routed;
}

function requireCommitted(outcome, phase) {
  if (!outcome.publicationReceipt) throw new Error(`${phase} returned no committed publication receipt`);
  if (!outcome.postcondition || outcome.postcondition.satisfied !== true) {
    throw new Error(`${phase} authoritative postcondition is not satisfied`);
  }
}

function reconcile(evidence, subjects) {
  if (!Array.isArray(evidence) || evidence.length !== subjects.length + 4) {
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

function authorPrompt(base, attempt, feedback) {
  if (feedback.length === 0) return `${base}\nLogical attempt: ${attempt}.`;
  return `${base}\nLogical attempt: ${attempt}. Repair these exact authoritative findings:\n- ${feedback.join("\n- ")}`;
}
