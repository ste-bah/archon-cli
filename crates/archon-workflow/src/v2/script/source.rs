//! Authoritative workflow.js source assembly.

use super::*;

pub fn script_source(harness_source: &str, script_args: Option<&serde_json::Value>) -> String {
    let normalized = normalize_workflow_export(harness_source);
    let v3_primitives = V3_PRIMITIVES_JS;
    let args_literal = script_args
        .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "undefined".to_string()))
        .unwrap_or_else(|| "undefined".to_string());
    format!(
        r#"
globalThis.args = {args_literal};

// Determinism prelude: workflow scripts must be replayable. Wall-clock and
// randomness are host concerns; pass timestamps via args.
delete Math.random;
Math.random = () => {{
  throw new Error("Math.random() is unavailable in workflow scripts: workflows must be deterministic");
}};
const __archonRealDate = Date;
globalThis.Date = new Proxy(__archonRealDate, {{
  apply() {{
    throw new Error("Date() is unavailable in workflow scripts: pass timestamps via args");
  }},
  construct(target, argumentList) {{
    if (argumentList.length === 0) {{
      throw new Error("new Date() without arguments is unavailable in workflow scripts: pass timestamps via args");
    }}
    return new target(...argumentList);
  }},
  get(target, property, receiver) {{
    if (property === "now") {{
      return () => {{
        throw new Error("Date.now() is unavailable in workflow scripts: pass timestamps via args");
      }};
    }}
    return Reflect.get(target, property, receiver);
  }},
}});

{normalized}

const __archonW = Object.freeze({{
  agent: (id, options = {{}}) => __archonCall("agent", id, undefined, options),
  implementation: (id, options = {{}}) => __archonCall("implementation", id, undefined, options),
  fanout: (id, source, options = {{}}) => __archonCall("fanout", id, source, options),
  parallel: (id, source, options = {{}}) => __archonCall("parallel", id, source, options),
  tool: (id, options = {{}}) => __archonCall("tool", id, undefined, options),
  hostCommand: (commandId, options = {{}}) => {{
    if (typeof commandId !== "string" || commandId.trim() === "") {{
      throw new Error("hostCommand requires a non-empty command capability id");
    }}
    if (options === null || typeof options !== "object" || Array.isArray(options)) {{
      throw new Error("hostCommand options must be an object");
    }}
    const stdin = options.stdin === undefined ? null : options.stdin;
    if (stdin !== null && typeof stdin !== "string") {{
      throw new Error("hostCommand stdin must be a string or null");
    }}
    __archonHostCommandSeq += 1;
    return __archonCall(
      "hostCommand",
      `hostCommand#${{__archonHostCommandSeq}}`,
      undefined,
      {{ ...options, commandId, stdin }}
    );
  }},
  // #189 Phase 4: the real tool registry, so a script can read or grep a file
  // without spending a model round-trip on it. Separate from `tool` above,
  // which reaches three workflow-internal pseudo-tools and nothing else.
  runTool: (name, input = {{}}) => {{
    if (typeof name !== "string" || name.trim() === "") {{
      throw new Error("runTool requires a tool name, e.g. runTool('Read', {{ file_path: '...' }})");
    }}
    // Sequential rather than random: the determinism rule above bans
    // Math.random, and two calls to the same tool still need distinct ids or
    // the pending-call set collapses them into one.
    __archonToolSeq += 1;
    return __archonCall("runTool", `${{name}}#${{__archonToolSeq}}`, undefined, {{ name, input }});
  }},
  checkpoint: (id, options = {{}}) => __archonCall("checkpoint", id, undefined, options),
  saveArtifact: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("saveArtifact", id, sourceOrOptions, options),
  requireArtifact: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("requireArtifact", id, sourceOrOptions, options),
  reduce: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("reduce", id, sourceOrOptions, options),
  qualityGate: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("qualityGate", id, sourceOrOptions, options),
  humanGate: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("humanGate", id, sourceOrOptions, options),
  finalReport: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("finalReport", id, sourceOrOptions, options),
}});

function __archonMaybeSourceCall(method, id, sourceOrOptions, options) {{
  if (options === undefined) {{
    return __archonCall(method, id, undefined, sourceOrOptions || {{}});
  }}
  return __archonCall(method, id, sourceOrOptions, options || {{}});
}}

// Every host call registers synchronously and deregisters on completion. A
// workflow that returns while calls are pending dropped real work on the
// floor (fire-and-forget async): fail closed, naming the dropped calls.
const __archonPendingCalls = new Set();

// Distinguishes repeat calls to the same tool (#189 Phase 4).
let __archonToolSeq = 0;
let __archonHostCommandSeq = 0;

async function __archonCall(method, id, source, options) {{
  if (typeof id !== "string" || id.trim() === "") {{
    throw new Error(`w.${{method}} requires a non-empty string id`);
  }}
  const payload = {{ id, options: options || {{}} }};
  if (source !== undefined) {{
    payload.source = source;
  }}
  const pendingKey = `${{method}}:${{id}}`;
  __archonPendingCalls.add(pendingKey);
  try {{
    const json = await __archonHost(method, JSON.stringify(payload));
    return JSON.parse(json);
  }} finally {{
    __archonPendingCalls.delete(pendingKey);
  }}
}}

{v3_primitives}

async function __archonRun() {{
  if (typeof workflow !== "function") {{
    throw new Error("workflow.js must export or define function workflow(w)");
  }}
  const meta = typeof __workflowMeta !== "undefined" ? __workflowMeta : undefined;
  const api = meta ? __archonPrimitives(__archonW) : __archonW;
  if (meta) {{
    // Top-level Claude Code scripts use the primitives as bare globals.
    globalThis.agent = api.agent;
    globalThis.agents = api.agents;
    globalThis.phase = api.phase;
    globalThis.log = api.log;
    globalThis.pipeline = api.pipeline;
    globalThis.adversarialReview = api.adversarialReview;
    globalThis.coverageAudit = api.coverageAudit;
    globalThis.remediateFindings = api.remediateFindings;
    globalThis.remediationBudget = api.remediationBudget;
    globalThis.accepted = api.accepted;
    globalThis.usable = api.usable;
    globalThis.outcomesOf = api.outcomesOf;
    globalThis.w = api.w;
    // #189 Phase 4. Taken from `__archonW` rather than `api` because the
    // primitives wrapper builds its own object and does not forward keys it
    // does not know about — reading it from there would silently define
    // `tool` as undefined.
    globalThis.tool = __archonW.runTool;
    globalThis.readFile = (file_path) => __archonW.runTool("Read", {{ file_path }});
    globalThis.grepFiles = (pattern, options = {{}}) => __archonW.runTool("Grep", {{ pattern, ...options }});
    globalThis.globFiles = (pattern, options = {{}}) => __archonW.runTool("Glob", {{ pattern, ...options }});
    globalThis.bash = (command, options = {{}}) => __archonW.runTool("Bash", {{ command, ...options }});
  }}
  const result = await workflow(api);
  if (meta && globalThis.__archonMarkers) {{
    // phase()/log() markers need no await in scripts; the runner flushes
    // them so they are journaled before completion.
    await Promise.all(globalThis.__archonMarkers);
  }}
  if (__archonPendingCalls.size > 0) {{
    const dropped = [...__archonPendingCalls].join(", ");
    throw new Error(`workflow returned while ${{__archonPendingCalls.size}} host call(s) were still pending (${{dropped}}); await every agent call — fire-and-forget drops real work`);
  }}
  return JSON.stringify(result ?? null);
}}

__archonRun()
"#
    )
}
