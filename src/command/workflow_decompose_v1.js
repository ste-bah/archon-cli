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

  const acceptance = await authorCandidate(w, {
    phase: "acceptance",
    author: authorAcceptanceEntries,
    capability: "freeze-acceptance",
    attempts: ACCEPTANCE_ATTEMPTS,
    retryScopes: new Set(["candidate_artifact"]),
    prompt: () => [
      "Author exactly one acceptance entry identified below, not the whole contract.",
      `Read the PRD at ${args.prdPath}; use repository source under ${args.projectRoot} only to verify real test names and paths. Never descend into: ${excludedDirs()}.`,
      "Return one JSON object with id, criterion, check, gap_permitted, judgment. The check may use either example shape below; do not return the enclosing contract.",
      ACCEPTANCE_SHAPE,
      "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges every entry and validates the assembled contract together.",
      "Use the exact supplied id. Criterion and judgment are host-owned placeholders. Set gap_permitted only if the PRD permits that criterion to remain a documented gap.",
      "Your entire reply must be the entry itself: the raw JSON object, starting with { and ending with }. Emit no prose, no explanation, no headings and no Markdown code fences before or after it.",
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
      `Read the PRD at ${args.prdPath}, the task root at ${args.taskRoot}, and repository source you need. Never descend into any directory named: ${excludedDirs()}. Those hold dependencies, build output and earlier runs' evidence, and are the overwhelming majority of files under that root. Stop reading once you can name what your entries assert.`,
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
        `Read the PRD at ${args.prdPath}, the frozen chain under ${args.taskRoot}, and repository source you need. Never descend into any directory named: ${excludedDirs()}. Those hold dependencies, build output and earlier runs' evidence, and are the overwhelming majority of files under that root. Stop reading once you can name what your entries assert. The project MCP configuration named below sits at the project root itself, not inside any excluded directory, and must still be read.`,
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

// Directory names the host says are not worth reading, joined for a prompt.
// Sourced from args so the engine's canonical list stays the single definition.
function excludedDirs() {
  const list = Array.isArray(args.excludedDirs) ? args.excludedDirs : [];
  return list.length > 0 ? list.join(', ') : '.git, node_modules, target';
}

async function authorCandidate(w, policy) {
  let feedback = [];
  const history = [];
  let bestCommitted = null;
  let bestFindings = Infinity;
  let call = 0;
  let attempt = 0;
  let operational = 0;
  let packagingRefunds = 0;
  let acceptanceRefusals = 0;
  const authorState = { entries: new Map(), retryIds: null };
  while (attempt < policy.attempts) {
    call += 1;
    const prompt = authorPrompt(policy.prompt(), attempt + 1, feedback, history);
    const authored = policy.author
      ? await policy.author(w, prompt, call, authorState)
      : await w.agent(`${policy.phase}-author-${call}`, {
          task: prompt, tier: "planner", resultMode: "rawOutcome"
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
      // A publication receipt means the whole set reached validation. Unknown
      // scope or an uncommitted candidate cannot certify unaffected siblings.
      authorState.retryIds = outcome.publicationReceipt && repair.every(f => ids.has(f.subject))
        ? new Set(repair.map(f => f.subject)) : null;
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

async function runSetGate(w, capability) {
  const outcome = await w.hostCommand(capability, { stdin: null });
  // Set-level skeleton findings shadow-mark the run and continue; a body
  // finding here is a first appearance after Phase C and stops the run.
  const routed = routeFindings(outcome, new Set(), new Set(["skeleton"]));
  if (routed.fatal.length > 0) {
    throw new Error(`${capability} stopped: ${routed.fatal.join(" | ")}`);
  }
  if (args.gateMode === "enforce" && routed.all.length > 0) {
    throw new Error(`${capability} found enforce-policy defects: ${routed.all.join(" | ")}`);
  }
  requireCommitted(outcome, capability);
  return outcome;
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
  const routed = { retry: [], fatal: [], inherited: [], shadow: [], all: [] };
  for (const finding of findings) {
    const text = typeof finding.text === "string" ? finding.text : "unnamed policy finding";
    const scope = finding.remediation_scope;
    routed.all.push(text);
    if (scope === "prd_input" || scope === "operational") routed.fatal.push(text);
    else if (scope === "inherited_predecessor") routed.inherited.push(text);
    else if (retryScopes.has(scope)) routed.retry.push(text);
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
    const lines = earlier.map((entry) => `attempt ${entry.attempt}: ${entry.findings.join("; ")}`);
    prompt += `\nEarlier attempts in this phase already triggered the following. Satisfy every one of them at once; repairing the finding above by reverting an earlier repair will not converge:\n- ${lines.join("\n- ")}`;
  }
  return prompt;
}

// Completed entries survive a sibling's incomplete reply. Assembly and validation
// belong to freeze-acceptance, not to a model or an unchecked JSON concatenation.
// The reply SHOULD be a bare JSON object; sometimes it is fenced or preceded by
// prose. A bare JSON.parse turns that formatting slip into a spent attempt, and
// with ACCEPTANCE_ATTEMPTS of them one entry can exhaust the whole budget while
// every reply carried a usable object. Run wf-cddf8426 died exactly that way:
// AC-AHDM-001 "exhausted 6 replies" when three were ```json-fenced objects and
// three were prose that ended in one.
//
// Take the outermost {...}. Anything that still fails to parse is genuinely
// malformed and retries as before.
function extractJsonObject(text) {
  const raw = String(text || "").trim();
  const fenced = raw.match(/```(?:json)?\s*([\s\S]*?)```/);
  const body = (fenced ? fenced[1] : raw).trim();
  const first = body.indexOf("{");
  const last = body.lastIndexOf("}");
  return first >= 0 && last > first ? body.slice(first, last + 1) : body;
}

async function authorAcceptanceEntries(w, prompt, round, state = { entries: new Map(), retryIds: null }) {
  const criteria = args.acceptanceCriteria;
  if (!criteria || Object.keys(criteria).length === 0) throw new Error("host acceptanceCriteria are missing");
  const ids = Object.keys(criteria).sort();
  const pending = ids.filter(id => !state.entries.has(id) || state.retryIds === null || state.retryIds.has(id));
  const cap = Number.isSafeInteger(args.authorMaxParallelism) && args.authorMaxParallelism > 0
    ? args.authorMaxParallelism : 1;
  for (let start = 0; start < pending.length; start += cap) {
    const batch = pending.slice(start, start + cap);
    const prior = ids.filter(id => state.entries.has(id) && !pending.includes(id))
      .concat(pending.slice(0, start)).map(id => state.entries.get(id));
    const results = await Promise.all(batch.map(async id => {
      for (let retry = 1; retry <= ACCEPTANCE_ATTEMPTS; retry++) {
        const result = await w.agent(`acceptance-author-${id}-${round * ACCEPTANCE_ATTEMPTS + retry}`, {
          task: `${prompt}\nAuthor ONLY entry ${id}: ${criteria[id]}\nAll criterion IDs and text (for consistency): ${JSON.stringify(criteria)}\nPreviously completed entries: ${JSON.stringify(prior)}`,
          tier: "planner", resultMode: "rawOutcome"
        });
        if (result.dry_run === true) return {entry:{id}};
        if (result.status === "failed") return {failure:result};
        if (result.stopReason !== "end_turn" || !result.content) continue;
        try {
          const entry = JSON.parse(extractJsonObject(result.content));
          if (entry && entry.id === id) return {entry};
        } catch (_) { /* Retry only this malformed entry. */ }
      }
      return {failure:{status:"failed",summary:`acceptance entry ${id} exhausted ${ACCEPTANCE_ATTEMPTS} replies`}};
    }));
    // All started calls settle before return; never abandon a sibling agent.
    for (const result of results) if (result.entry) state.entries.set(result.entry.id, result.entry);
    const failure = results.find(result => result.failure);
    if (failure) return failure.failure;
  }
  return {status:"accepted",stopReason:"end_turn",content:JSON.stringify({entries:ids.map(id => state.entries.get(id))})};
}
