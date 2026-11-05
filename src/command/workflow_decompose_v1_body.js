// Body author policy, concatenated into the fixed script scope.
const BODY_AUTHOR_MARKER = "Author the complete TASK body for host-frozen task_id ";
const BODY_FROZEN_RULE = "The host stamps implements, source_sections, depends_on, blocks and deliverable_contracts from the frozen skeleton; read them there";

// One body policy, whether the body is authored fresh after the skeleton or
// re-authored because the set gate named it (Issue-46). `initialFeedback`
// carries the set gate's exact finding texts into the first attempt.
function bodyPolicy(subject, initialFeedback) {
  return {
    phase: `body-${subject.taskId}`,
    capability: "land-task-body",
    target: { taskId: subject.taskId, fileName: subject.fileName },
    retryScopes: new Set(["candidate_artifact", "body"]),
    shadowScopes: new Set(["skeleton"]),
    initialFeedback,
    prompt: () => [
      `${BODY_AUTHOR_MARKER}${subject.taskId}.`,
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
      BODY_FROZEN_RULE,
      "Name every frozen implements id (obligation or constraint) in a body sentence outside the yaml block that also names the file, symbol or command that keeps or proves it.",
      ...(typeof args.acceptanceCheckSitePathLine === "string" ? [args.acceptanceCheckSitePathLine] : []),
      ...constraintContext("body"),
      ...sectionContext(subject),
      `Read the project MCP configuration at ${args.projectRoot}/.mcp.json and match this task's PRD obligations to its exact permitted tool names.`,
      "Declare only task-specific invocation obligations: every declared tool must actually be called and reported in commands_run; use fully qualified mcp__server__tool names for MCP calls.",
      "An MCP deliverable cannot declare no MCP tools. List the exact MCP calls and inputs in Focused Tests; shell commands and recorded fixtures alone do not exercise MCP.",
      "Every MCP tool called by a Focused Tests command must appear in required_tools, using its exact project-permitted name from .mcp.json.",
      "HTTP/service providers are not MCP tools. Declare required environment keys only when live execution requires them; preserve explicitly permitted no-credential/unavailable paths. Never copy the ambient project toolchain into every task.",
      "Every test file run by Focused Tests must be declared by an owning task in Files Expected to Change, a shared-append target or a deliverable contract (a declared directory may cover it); declare a test file in the Files Expected to Change of the task that owns it.",
      "Files Forbidden to Change entries must be literal paths, directories, basenames or globs, not prose descriptions.",
      "After the yaml block, use Markdown headings; include a `## Focused Tests` section whose entries are runnable commands.",
      "The yaml block is part of the file: close it with a ``` line of its own before the first Markdown heading.",
      "Generate large or repetitive fixture data with code rather than writing it literally.",
      "Your entire reply must be the TASK file itself, as raw UTF-8 Markdown.",
      "Wrap the reply in no outer code fence, and add no commentary before or after it.",
      "Do not run commands or write files."
    ].join("\n")
  };
}

// The durable author replies and gate outcomes already live in the run journal.
// Like progress, this index is rebuilt by replay, including across pool pauses
// and set-gate retries. No score can stand in for a committed gate outcome.
const BODY_CANDIDATES = new Map();
const BODY_PUBLISHED = new Map();

function retainBodyCandidate(phase, body, findings, history = []) {
  findings = bodyRetryFindings(findings);
  if (findings.length === 0) return;
  const measure = { ...attemptMeasure(findings), count: findings.length };
  const score = findings.length;
  const prior = BODY_CANDIDATES.get(phase);
  if (!prior || prior.body === body || !isBetter(prior.measure, measure)) {
    BODY_CANDIDATES.set(phase, { body, findings: findings.slice(), score, measure, history: history.slice() });
  }
}

function publishBodyCandidate(phase, body) {
  BODY_PUBLISHED.set(phase, body);
  BODY_CANDIDATES.delete(phase);
}

// A set gate judges the published body, not an unlanded repair draft.
// Routing assigns these findings to this body whatever their original scope.
function retainSetBodyCandidate(subject, findings) {
  const phase = `body-${subject.taskId}`;
  const body = BODY_PUBLISHED.get(phase);
  BODY_CANDIDATES.delete(phase);
  if (body !== undefined) {
    retainBodyCandidate(phase, body, findings.map(f => ({ ...f, remediation_scope: "body" })));
  }
}

function dropBodyCandidate(subject) {
  const phase = `body-${subject.taskId}`;
  BODY_CANDIDATES.delete(phase);
  BODY_PUBLISHED.delete(phase);
}

function bodyRetryFindings(findings) {
  return [...new Map(findings.filter(f => ["body", "candidate_artifact"].includes(f.remediation_scope))
    .map(f => [findingKey(f), f])).values()];
}

function bodyRepairContext(policy, rung) {
  if (policy.capability !== "land-task-body") return null;
  const best = BODY_CANDIDATES.get(policy.phase);
  if (!best || best.findings.length === 0) return null;
  if (rung === 4) return {
    text: `\nRe-author a complete replacement from fresh context. Keep the subject contract. Previously seen defects:\n- ${best.findings.map(findingText).join("\n- ")}`,
    feedback: [], history: best.history
  };
  const instruction = "Correct the retained candidate below. Change only what its findings require and keep every other line exactly. Return the complete corrected file, never a patch.";
  const details = JSON.stringify({ findings: best.findings, blocking_findings: best.score });
  let reference;
  try {
    const body = typeof best.body === "string" ? best.body : null;
    const path = body === null ? best.body.path : utf8Bytes(body) > 16384 ? contextFile("txt", body).path : null;
    reference = path
      ? `Read the retained candidate as raw text at ${path} (host-written or frozen body context for this call: read it by its exact path, even under an excluded directory).`
      : `Retained candidate (raw text):\n${body}\nEnd retained candidate.`;
    reference += utf8Bytes(details) > 16384
      ? `\nRead exact gate findings (JSON) at ${contextFile("json", details).path} (host-written context: read this exact path).`
      : `\nExact gate findings (JSON):\n${details}`;
  } catch (error) {
    error.retainedBodyContext = true;
    error.authorContext = true;
    throw error;
  }
  return { text: `\n${instruction}\n${reference}`, feedback: best.findings.map(findingText), history: best.history };
}
