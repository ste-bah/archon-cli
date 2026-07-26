// v3 dialect support: the Claude-Code-style primitive layer injected into
// workflow scripts marked by `export const meta`, and the export
// normalization shared by every dialect. Split from script_helpers to
// respect the 500-line source ceiling.

const V3_PRIMITIVES_JS: &str = r#"
// v3 script dialect (marked by `export const meta`): Claude-Code-style
// primitives layered over the host API. Call ids derive deterministically
// from labels + ordinals so unchanged prefixes replay from cache.
function __archonPrimitives(w) {
  let ordinal = 0;
  let phaseIndex = 0;
  // phase()/log() are UI/journal markers: Claude Code scripts call them
  // without await. Their checkpoint promises are collected here and flushed
  // by the runner after the workflow returns, so they can neither be dropped
  // nor trip the pending-call guard.
  globalThis.__archonMarkers = [];
  const slug = (text) =>
    String(text).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "step";
  // Early UX guard mirroring the HOST rule (authoritative copy runs in the
  // dry-run recorder): literal repo-relative paths — no whitespace, no
  // traversal, not absolute, no globs. Extensionless files like Makefile are
  // valid. Write work requires a non-empty list.
  const assertPathList = (list, what, requireNonEmpty) => {
    if (requireNonEmpty && (!Array.isArray(list) || list.length === 0)) {
      throw new Error(`${what} must list at least one literal repo-relative file path for write work`);
    }
    for (const entry of list || []) {
      const bad =
        typeof entry !== "string" ||
        entry.trim() === "" ||
        /\s/.test(entry) ||
        entry.startsWith("/") ||
        entry.split("/").includes("..") ||
        /[*?\[]/.test(entry);
      if (bad) {
        throw new Error(`${what} entries must be literal repo-relative file paths (no whitespace, traversal, globs, or absolute paths; got: ${JSON.stringify(entry).slice(0, 120)})`);
      }
    }
  };
  const agent = async (prompt, opts = {}) => {
    if (typeof prompt !== "string" || prompt.trim() === "") {
      throw new Error("agent(prompt, opts) requires a non-empty prompt string");
    }
    ordinal += 1;
    const id = `${slug(opts.label || "agent")}-${ordinal}`;
    if (opts.write) {
      assertPathList(opts.targetFiles, "agent() targetFiles", true);
      const item = {
        item_id: id,
        canonical_task_ids: opts.taskIds || [],
        task: prompt,
        instructions: prompt,
        target_files: opts.targetFiles || [],
        focused_verification: opts.focusedTests || [],
        artifact_requirements: opts.artifacts || [],
        work_type: "implementation",
      };
      const writeOptions = {
        write: "worktree",
        itemKind: "implementation",
        tier: opts.tier || "coder",
        targetFilesFromItem: true,
        maxParallelism: 1,
        task: prompt,
      };
      // Only remediateFindings sets this. It marks work that a mandatory review
      // ASKED for, which the ordering rule must not confuse with work smuggled
      // in after the reviewers looked. The validator checks the contract's
      // claims against the actual plan, so declaring one buys nothing unless
      // the reduce calls it names really precede it and a verifier really
      // follows it.
      if (opts.remediationContract) writeOptions.remediationContract = opts.remediationContract;
      return await w.fanout(id, [item], writeOptions);
    }
    // Per-task verifiers (verify:true or focusedTests) run through the
    // HOST's verification-wave machinery: the wave id prefix grants command
    // execution and attaches every focused-verification guard (zero-test and
    // zero-command demotion, outcome normalization). commands_run is
    // agent-reported — the demotions are what keep it honest. The
    // adversarial reviewer stays a plain read-only agent by design.
    if (opts.verify === true || (Array.isArray(opts.focusedTests) && opts.focusedTests.length > 0)) {
      const item = {
        item_id: `${id}-check`,
        canonical_task_ids: opts.taskIds || [],
        task: prompt,
        instructions: prompt,
        focused_verification: opts.focusedTests || [],
        artifact_requirements: opts.artifacts || [],
      };
      if (item.focused_verification.length === 0) {
        // Goal-oriented verifier: the agent chooses its own commands
        // in-session. The prompt rides as verification_requirements (what to
        // prove); host item normalization copies it into focused_verification
        // to satisfy the wave metadata contract. Fail-closed backstop: an
        // accepted outcome recording no successful command execution is
        // demoted by the host (commands_run is agent-reported, so absence of
        // execution must never verify anything).
        item.verification_requirements = [prompt];
      }
      const verifyOptions = {
        tier: opts.tier || "coder",
        itemKind: "focused_verification",
        task: prompt,
      };
      if (opts.remediationContract) verifyOptions.remediationContract = opts.remediationContract;
      return await w.parallel(`verification-wave-${id}`, [item], verifyOptions);
    }
    return await w.agent(id, {
      tier: opts.tier || "coder",
      task: prompt,
      targetFiles: opts.targetFiles || [],
    });
  };
  // Batch form: N specs through ONE host fanout/parallel call, so the HOST
  // controls safe concurrency (worktree isolation, build-lock serialization).
  const agents = async (specs, opts = {}) => {
    if (!Array.isArray(specs) || specs.length === 0) {
      throw new Error("agents(specs, opts) requires a non-empty array of {prompt, label, ...} specs");
    }
    ordinal += 1;
    const id = `${slug(opts.label || "agents")}-${ordinal}`;
    const cargoish = specs.some((spec) =>
      [spec.prompt, ...(spec.focusedTests || [])].join(" ").includes("cargo "));
    const items = specs.map((spec, index) => {
      if (typeof spec.prompt !== "string" || spec.prompt.trim() === "") {
        throw new Error("every agents() spec requires a non-empty prompt");
      }
      if (opts.write) {
        assertPathList(spec.targetFiles, "agents() spec targetFiles", true);
      }
      return {
        item_id: `${id}-${slug(spec.label || `item-${index + 1}`)}`,
        canonical_task_ids: spec.taskIds || [],
        task: spec.prompt,
        instructions: spec.prompt,
        target_files: spec.targetFiles || [],
        focused_verification: spec.focusedTests || [],
        artifact_requirements: spec.artifacts || [],
        work_type: opts.write ? "implementation" : "verification",
      };
    });
    if (opts.write) {
      return await w.fanout(id, items, {
        write: "worktree",
        itemKind: "implementation",
        tier: opts.tier || "coder",
        targetFilesFromItem: true,
        maxParallelism: cargoish ? 1 : opts.maxParallelism,
        task: opts.task || "Execute every item in this batch.",
      });
    }
    return await w.parallel(id, items, {
      tier: opts.tier || "coder",
      maxParallelism: cargoish ? 1 : opts.maxParallelism,
      task: opts.task || "Execute every item in this batch.",
    });
  };
  const phase = (title, body) => {
    phaseIndex += 1;
    const marker = w.checkpoint(`phase-${phaseIndex}-${slug(title)}`, {
      task: `Phase: ${String(title).slice(0, 200)}`,
    });
    globalThis.__archonMarkers.push(marker);
    // Three valid styles: bare `phase('T')` (Claude Code form, no await
    // needed), `await phase('T')`, or `await phase('T', async () => {...})`
    // which runs and awaits the body and returns its result — silently
    // ignoring a body function would drop entire phases of real work.
    if (typeof body === "function") {
      return (async () => {
        await marker;
        return await body();
      })();
    }
    return marker;
  };
  const log = (message) => {
    ordinal += 1;
    const marker = w.checkpoint(`log-${ordinal}`, {
      task: `Log: ${String(message).slice(0, 400)}`,
    });
    globalThis.__archonMarkers.push(marker);
    return marker;
  };
  const pipeline = async (items, stages) => {
    if (!Array.isArray(items) || !Array.isArray(stages)) {
      throw new Error("pipeline(items, stages) requires an items array and a stages array of async functions");
    }
    const results = [];
    for (const item of items) {
      let current = item;
      for (const stage of stages) {
        current = await stage(current);
      }
      results.push(current);
    }
    return results;
  };
  // Mandatory reviews as a RUNTIME primitive: fanout one critic reviewer per
  // accepted task (the map — bounded, so a large deliverable can never overflow
  // one context) then a single critic reduce over the map findings (the cross-
  // task pass). The author calls one line; it never authors the map/reduce shape
  // itself. Read-only; the reduce feeds the accounting field named by `kind`.
  const findingsFrom = (env) => {
    const direct = env && env.data && env.data.findings;
    if (Array.isArray(direct)) return direct;
    const nested = env && env.result && env.result.data && env.result.data.findings;
    return Array.isArray(nested) ? nested : [];
  };
  const reviewMapReduce = async (label, kind, mapTask, reduceTask, acceptedTaskIds, evidenceFor) => {
    const ids = Array.isArray(acceptedTaskIds) ? acceptedTaskIds : [];
    const map = await w.parallel(`${label}-map`, ids.map((taskId) => ({
      item_id: `review-${slug(taskId)}`,
      canonical_task_ids: [taskId],
      task: mapTask,
      evidence: (typeof evidenceFor === "function" ? evidenceFor(taskId) : []),
    })), {
      tier: "critic",
      itemKind: "review_map",
      maxParallelism: 4,
      task: mapTask,
      reviewContract: { version: 1, kind, stage: "map", findingsPath: "data.findings", itemTaskIdsPath: "canonical_task_ids", maxFindingsPerItem: 25 },
    });
    const reduce = await w.reduce(`${label}-reduce`, { findings: findingsFrom(map) }, {
      tier: "critic",
      task: reduceTask,
      reviewContract: { version: 1, kind, stage: "reduce_final", sourceMapCallIds: [`${label}-map`], preserveMapFindings: true, findingsPath: "data.findings", accountingField: kind, maxInputBytes: 48000 },
    });
    return findingsFrom(reduce);
  };
  const adversarialReview = async (acceptedTaskIds, opts = {}) =>
    reviewMapReduce(
      "adversarial-review",
      "adversarial_findings",
      "You did NOT do this work — be suspicious. Try to FALSIFY this accepted task using only its own claims and the bounded evidence supplied. Return data.findings as compact structured findings (max 25).",
      "Preserve every map finding verbatim, then add any cross-task contradictions you can see across the map findings. Return data.findings.",
      acceptedTaskIds,
      opts.evidenceFor,
    );
  const coverageAudit = async (acceptedTaskIds, opts = {}) =>
    reviewMapReduce(
      "coverage-audit",
      "uncovered_requirements",
      "Compare this accepted task against the source requirements it claims to satisfy. Return data.findings for any requirement it appears NOT to cover.",
      "Preserve every map coverage finding verbatim, deduplicate only by exact finding identity, then add cross-task uncovered-requirement findings. Return data.findings.",
      acceptedTaskIds,
      opts.evidenceFor,
    );
  // Group review findings by the canonical task id(s) they name. Reviewers emit
  // ids under the review contract's itemTaskIdsPath; accept the common aliases
  // so a reducer that renames the field does not silently drop the finding.
  const findingsByTask = (findings) => {
    const grouped = {};
    const unassigned = [];
    const list = Array.isArray(findings) ? findings : [];
    for (const finding of list) {
      const raw = finding && (finding.canonical_task_ids || finding.task_ids || finding.taskIds
        || (finding.task_id ? [finding.task_id] : []) || []);
      const ids = (Array.isArray(raw) ? raw : [raw]).filter(Boolean);
      if (ids.length === 0) { unassigned.push(finding); continue; }
      for (const id of ids) {
        if (!grouped[id]) grouped[id] = [];
        grouped[id].push(finding);
      }
    }
    return { grouped, unassigned };
  };
  // Act on review findings instead of only reporting them.
  //
  // The mandatory reviews are the last stages before final accounting — they can
  // only judge work once every task is done — so historically their findings were
  // terminal output and nothing consumed them: a run could surface ~96 verified
  // findings and exit having fixed none. This runs a BOUNDED fix+re-verify pass
  // over the findings that name a task, and returns what is still outstanding so
  // the caller records it honestly. It never forces acceptance: an unresolved
  // finding stays unresolved, and findings naming no task are returned untouched
  // rather than quietly dropped.
  const remediateFindings = async (findings, opts = {}) => {
    // Local envelope helpers: the author's own isAccepted/summarize live in the
    // authored script, not here, so the prelude must not depend on them.
    const acceptedEnvelope = (env) => {
      const status = String((env && (env.status || (env.result && env.result.status))) || "").toLowerCase();
      return ["accepted", "passed", "ok", "succeeded", "success", "complete", "completed", "verified_noop", "noop"].indexOf(status) >= 0;
    };
    const summarizeEnvelope = (env) =>
      String((env && (env.summary || (env.result && env.result.summary))) || "no summary").slice(0, 300);
    const { grouped, unassigned } = findingsByTask(findings);
    const taskIds = Object.keys(grouped);
    const maxRounds = Math.max(1, Number(opts.maxRounds) || 2);
    // The reduces whose findings this pass acts on. Naming them is what lets the
    // validator tell review-ordered remediation apart from work hidden from the
    // reviewers: it confirms these calls really are final reduces and really do
    // precede every remediation call below.
    const sourceReduceCallIds = Array.isArray(opts.sourceReduceCallIds) && opts.sourceReduceCallIds.length > 0
      ? opts.sourceReduceCallIds
      : ["adversarial-review-reduce", "coverage-audit-reduce"];
    const contractFor = (stage, taskId, round) => ({
      version: 1,
      stage,
      taskId,
      round,
      maxRounds,
      sourceReduceCallIds,
    });
    const resolved = [];
    const unresolved = [];
    for (const taskId of taskIds) {
      const own = grouped[taskId];
      const verbatim = JSON.stringify(own).slice(0, 6000);
      const context = typeof opts.taskFileFor === "function" ? opts.taskFileFor(taskId) : "";
      const targetFiles = typeof opts.targetFilesFor === "function" ? opts.targetFilesFor(taskId) : undefined;
      let fix = null;
      let check = null;
      for (let round = 1; round <= maxRounds; round += 1) {
        fix = await agent(
          `Post-review remediation for ${taskId}${context ? ` per ${context}` : ""}. A read-only review of ALREADY-ACCEPTED work raised the findings below. Fix exactly what they name; do not re-argue them. If a finding is factually wrong, say so with the evidence that disproves it rather than editing around it. Findings (verbatim):\n${verbatim}\nProve every fix with tests you run yourself.`,
          {
            label: `review-remediate-${slug(taskId)}-${round}`,
            write: true,
            taskIds: [taskId],
            targetFiles,
            remediationContract: contractFor("remediate", taskId, round),
          },
        );
        check = await agent(
          `You did NOT do this remediation — be suspicious of its self-report. These review findings were raised against ${taskId}:\n${verbatim}\nInspect the actual code and artifacts and run whatever checks YOU judge prove each finding is genuinely resolved (or was invalid).`,
          {
            label: `review-verify-${slug(taskId)}-${round}`,
            verify: true,
            taskIds: [taskId],
            remediationContract: contractFor("verify", taskId, round),
          },
        );
        if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
      }
      if (acceptedEnvelope(fix) && acceptedEnvelope(check)) {
        resolved.push({ taskId, findingCount: own.length });
      } else {
        unresolved.push({ taskId, findingCount: own.length, reason: summarizeEnvelope(check) });
      }
    }
    return { resolved, unresolved, unassigned };
  };
  return Object.freeze({ agent, agents, phase, log, pipeline, adversarialReview, coverageAudit, remediateFindings, w });
}
"#;

fn normalize_workflow_export(source: &str) -> String {
    let mut normalized = source.trim().to_string();
    // v3 dialect marker: `export const meta` becomes a plain const plus the
    // global flag __archonRun uses to hand the script the primitive API.
    if let Some(offset) = workflow_meta_marker_offset(&normalized) {
        normalized.replace_range(offset..offset + "export const meta".len(), "const meta");
        // The genuine Claude Code script shape is TOP-LEVEL code after the
        // meta export — no wrapper function. Wrap everything after the meta
        // statement so top-level `await` and `return` become legal, with the
        // primitives available as globals.
        if !has_workflow_function_declaration(&normalized) {
            let body_start = statement_end_offset(&normalized, offset);
            let body = normalized.split_off(body_start);
            normalized.push_str("\nasync function workflow() {\n");
            normalized.push_str(&body);
            normalized.push_str("\n}");
        }
        normalized.insert_str(0, "globalThis.__workflowMeta = true;\n");
    }
    // QuickJS evaluates non-module source: neutralize the default export
    // wherever it appears (v3 scripts put `export const meta` first).
    for (from, to) in [
        (
            "export default async function workflow",
            "async function workflow",
        ),
        ("export default function workflow", "function workflow"),
        ("export default async function(", "async function workflow("),
        ("export default function(", "function workflow("),
    ] {
        if normalized.contains(from) {
            normalized = normalized.replacen(from, to, 1);
            break;
        }
    }
    normalized
        .replace("export default workflow;", "")
        .replace("export default workflow", "")
}

fn has_workflow_function_declaration(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        [
            "export default async function workflow",
            "export default function workflow",
            "async function workflow",
            "function workflow",
        ]
        .iter()
        .any(|declaration| line.starts_with(declaration))
    })
}

/// Offset just past the end of the statement starting at `start` — the
/// balanced close of its first `{...}` block plus an optional trailing `;`.
/// Quote- and escape-aware so braces inside meta strings don't miscount.
fn statement_end_offset(source: &str, start: usize) -> usize {
    let bytes = &source[start..];
    let Some(open) = bytes.find('{') else {
        return source.len();
    };
    let mut depth = 0usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (offset, ch) in bytes[open..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string.is_some() => escaped = true,
            '"' | '\'' | '`' => match in_string {
                Some(quote) if quote == ch => in_string = None,
                None => in_string = Some(ch),
                _ => {}
            },
            '{' if in_string.is_none() => depth += 1,
            '}' if in_string.is_none() => {
                depth -= 1;
                if depth == 0 {
                    let mut end = start + open + offset + ch.len_utf8();
                    if source[end..].starts_with(';') {
                        end += 1;
                    }
                    return end;
                }
            }
            _ => {}
        }
    }
    source.len()
}

fn workflow_meta_marker_offset(source: &str) -> Option<usize> {
    let marker = "export const meta";
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let declaration = line.trim_start();
        if let Some(rest) = declaration.strip_prefix(marker)
            && rest
                .chars()
                .next()
                .is_none_or(|ch| ch.is_ascii_whitespace() || ch == '=')
        {
            return Some(offset + line.len() - declaration.len());
        }
        offset += line.len();
    }
    None
}
