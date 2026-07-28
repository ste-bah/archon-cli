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
  // Findings out of an envelope, whichever shape it has.
  //
  // A reduce returns ONE envelope carrying data.findings. A map is a fanout:
  // its envelope has no data.findings at all — every reviewer's findings sit in
  // its own branch outcome. Reading only the top level therefore silently
  // returned [] for every map, so the reduce was handed nothing and truthfully
  // reported "no map findings were present" while 29 real findings sat in the
  // branches. The mandatory review then read as clean, preserveMapFindings was
  // vacuously satisfied at 0 of 0, and the remediation loop had nothing to act
  // on — a false all-clear that looks exactly like a good run.
  const findingsFrom = (env) => {
    const direct = env && env.data && env.data.findings;
    if (Array.isArray(direct)) return direct;
    const nested = env && env.result && env.result.data && env.result.data.findings;
    if (Array.isArray(nested)) return nested;
    const outcomes = (env && env.data && env.data.outcomes)
      || (env && env.result && env.result.data && env.result.data.outcomes);
    if (!Array.isArray(outcomes)) return [];
    const collected = [];
    for (const outcome of outcomes) {
      const branch = outcome && outcome.result && outcome.result.data && outcome.result.data.findings;
      if (Array.isArray(branch)) collected.push(...branch);
    }
    return collected;
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
  // Attempt budget that follows PROGRESS rather than a flat count.
  //
  // A flat cap funds the stuck task exactly as much as the one closing gaps:
  // observed live, a task reported 5 gaps, closed 1, then reported the same 4
  // twice more and was cut off with real work outstanding, while another
  // burned its whole budget rediscovering one mechanical remedy.
  //
  // Progress is measured against the FIRST attempt's gap set, not the previous
  // attempt's. Incidental gaps churn (one run saw test-filter-zero-match become
  // zero-test-noise between attempts) and a consecutive diff reads that as
  // movement; anything absent from the baseline can never earn budget, so churn
  // buys nothing and the engine never has to judge which gaps are substantive —
  // a judgement it cannot make domain-neutrally.
  //
  // Gap ids come from the VERIFIER's envelope. Measured across one run,
  // verification-sourced ids were 17 clean slugs with none suffixed, while
  // write-sourced ids were 24 with 17 carrying a branch suffix
  // (invalid_write_branch_output_<item>) that could never match across
  // attempts and would read as perpetual churn. Choosing the source removes
  // the normalisation problem instead of solving it.
  const remediationBudget = (opts = {}) => {
    const base = Math.max(1, Number(opts.baseAttempts) || 3);
    const hardCap = Math.max(base, Number(opts.hardCap) || 6);
    // An attempt whose schema repair failed while its patch nonetheless LANDED
    // produced real work and no verdict. Charging it to the task discards work
    // that is already on disk — the third shape of "an attempt burned by
    // something that says nothing about the work", after the 520 and the
    // verifier timeout.
    //
    // Granted as one EXTRA attempt rather than an un-counted one, because the
    // loop that owns `attempt` is the GENERATED script's and increments
    // unconditionally. A refund is only expressible here, as added funding.
    // For a bounded loop the two are behaviourally identical.
    //
    // Bounded to once per task, and the bound IS the safety argument: schema
    // repair already retries under its own cap, so an unbounded exemption
    // trades a burned attempt for a hung task — strictly worse. An agent that
    // emits garbage and lands a patch every single time must still run out.
    const maxSchemaRefunds = Number.isFinite(Number(opts.maxSchemaRefunds))
      ? Math.max(0, Number(opts.maxSchemaRefunds))
      : 1;
    let schemaRefunds = 0;
    // Keyed on the host's TYPED marker, never on prose. The host sets it only
    // where the worktree was compared against the declared baseline; a bare
    // "files changed" test would count stray tool output as landed work and
    // refund an attempt that produced none.
    const schemaRefundable = (...envs) => {
      for (const env of envs) {
        if (!env) continue;
        let blob = "";
        try { blob = JSON.stringify(env); } catch (_) { continue; }
        if (blob.indexOf('"schema_repair_patch_landed":true') >= 0) return true;
      }
      return false;
    };
    let baseline = null;
    const gapIdsOf = (env) => {
      const out = new Set();
      const walk = (node) => {
        if (!node) return;
        if (Array.isArray(node)) { node.forEach(walk); return; }
        if (typeof node !== "object") return;
        if (Array.isArray(node.residual_gaps)) {
          for (const gap of node.residual_gaps) {
            const id = gap && gap.id;
            // A suffixed id embeds a branch or item name and cannot match
            // across attempts. Excluded rather than normalised: silently
            // "fixing" it would hide a source that should not be used here.
            if (typeof id === "string" && id && !/_(?:[a-z0-9]+-){2,}/.test(id)) out.add(id);
          }
        }
        for (const value of Object.values(node)) walk(value);
      };
      walk(env);
      return out;
    };
    let lastRemaining = null;
    return {
      // Called after each attempt with the VERIFIER envelope. Returns whether
      // another attempt is warranted.
      // `implEnv` is optional: a script generated before this argument existed
      // still runs, it simply cannot earn the schema refund. Checked on BOTH
      // envelopes because the observed TDL-020 failure was on the write half,
      // which never reaches the verifier envelope at all.
      shouldContinue(attempt, checkEnv, implEnv) {
        if (schemaRefunds < maxSchemaRefunds && schemaRefundable(implEnv, checkEnv)) {
          schemaRefunds += 1;
        }
        const funded = base + schemaRefunds;
        const ceiling = hardCap + schemaRefunds;
        const ids = gapIdsOf(checkEnv);
        if (baseline === null) {
          baseline = ids;
          lastRemaining = ids.size;
          // A verifier that named nothing gives nothing to measure against;
          // the flat budget still applies.
          return attempt < funded;
        }
        const remaining = [...baseline].filter((id) => ids.has(id)).length;
        // Recorded on EVERY call, including inside the base window. Updating it
        // only after the base attempts made a flat set look like progress: the
        // comparison fell back to the baseline size and read 3 -> 3 as 5 -> 3.
        const stillClosing = remaining < lastRemaining;
        lastRemaining = remaining;
        if (attempt < funded) return true;
        if (attempt >= ceiling) return false;
        // Extend only while the ORIGINAL diagnosis is still shrinking. A
        // plateau means attempts have stopped converging, which is when more
        // of them stop being worth buying.
        return stillClosing;
      },
    };
  };
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
    // A call the PROVIDER failed says nothing about the work. Spending a round
    // on it costs the task an attempt it never had, and when the failure lands
    // on the verifier it also strands an already-accepted fix as unverified —
    // a correct patch recorded as unresolved because its checker died. Both
    // were observed live: a 520 on a fix, a 1200s timeout on a check.
    //
    // The host already draws this line — is_write_branch_validation_error
    // excludes "agent transport failed" so it is not classified as a contract
    // violation. This applies the same distinction to remediation rounds.
    const transportFailure = (env) => {
      if (!env) return false;
      let blob = "";
      try { blob = JSON.stringify(env); } catch (_) { return false; }
      // Cancellation is a deliberate stop, not a provider failure. It shares
      // the Execution kind, so it must be excluded before the typed check or a
      // cancelled round would be refunded.
      if (blob.indexOf("cancelled") >= 0) return false;
      // Typed signal: BranchFailureKind::Execution is the host's own
      // classification for transport, timeout and rate-limit failures, and it
      // is a persisted enum rather than prose.
      if (blob.indexOf('"failure_kind":"execution"') >= 0) return true;
      // A call that fails WHOLESALE produces no branch outcomes and therefore
      // no failure_kind at all — observed on a 520, where the only evidence is
      // the summary text. Fall back to the exact markers the host itself
      // excludes from write-branch validation errors.
      return blob.indexOf("agent transport failed") >= 0
        || blob.indexOf("timed out after") >= 0;
    };
    // Tasks that exhausted their own remediation budget are ALSO unfinished work.
    // The reviews only inspect ACCEPTED tasks — they hunt false acceptance — so a
    // blocked task can never appear in their findings and would otherwise be
    // reported and abandoned. Fold each blocked task in as a finding naming
    // itself, so the same bounded pass gets one more attempt at it with
    // everything the run has learned since. Fixing false acceptance and finishing
    // acknowledged failure are the two halves of "no work silently abandoned".
    const blocked = Array.isArray(opts.blockedTasks) ? opts.blockedTasks : [];
    const blockedAsFindings = blocked
      .filter((entry) => entry && entry.taskId)
      .map((entry) => ({
        canonical_task_ids: [entry.taskId],
        id: `blocked-task-${slug(entry.taskId)}`,
        description: `This task exhausted its remediation budget without passing verification. Last verifier summary: ${String(entry.reason || "no summary")}`,
      }));
    const { grouped, unassigned } = findingsByTask([...(Array.isArray(findings) ? findings : []), ...blockedAsFindings]);
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
      // Transport retries have their own small budget so a sustained provider
      // outage cannot spin: they do not consume a round, but they are not free.
      let transportRetries = 0;
      const maxTransportRetries = 2;
      for (let round = 1; round <= maxRounds; ) {
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
        // A provider failure on either half retries without spending the
        // round. Retrying the fix would re-apply work that may already be on
        // disk, so when only the CHECK failed, re-run the check alone.
        if (transportFailure(fix) && transportRetries < maxTransportRetries) {
          transportRetries += 1;
          log(`transport failure on ${taskId} remediation; retrying without consuming round ${round}`);
          continue;
        }
        if (transportFailure(check) && transportRetries < maxTransportRetries) {
          transportRetries += 1;
          log(`transport failure verifying ${taskId}; re-running the check without consuming round ${round}`);
          check = await agent(
            `You did NOT do this remediation — be suspicious of its self-report. These review findings were raised against ${taskId}:\n${verbatim}\nInspect the actual code and artifacts and run whatever checks YOU judge prove each finding is genuinely resolved (or was invalid).`,
            {
              label: `review-verify-${slug(taskId)}-${round}r${transportRetries}`,
              verify: true,
              taskIds: [taskId],
              remediationContract: contractFor("verify", taskId, round),
            },
          );
        }
        if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
        round += 1;
      }
      if (acceptedEnvelope(fix) && acceptedEnvelope(check)) {
        resolved.push({ taskId, findingCount: own.length });
      } else {
        unresolved.push({ taskId, findingCount: own.length, reason: summarizeEnvelope(check) });
      }
    }
    return { resolved, unresolved, unassigned };
  };
  return Object.freeze({ agent, agents, phase, log, pipeline, adversarialReview, coverageAudit, remediateFindings, remediationBudget, w });
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

#[cfg(test)]
mod primitive_binding_tests {
    /// Every primitive the prelude exports must also be bound as a global.
    ///
    /// Authored (v3) scripts call these bare — `remediateFindings(...)`, not
    /// `api.remediateFindings(...)` — so a primitive that is exported from
    /// `__archonPrimitives` but missing from the globals block does not exist as
    /// far as the script is concerned. That shipped once: the findings-loop
    /// primitive was written, wired into the author reference, and passed every
    /// unit test, then killed a live run at dry-run pre-flight with
    /// `remediateFindings is not defined`.
    ///
    /// It is the same failure as a verifier that is never invoked and a
    /// primitive the validator forbids: the code is correct and unreachable.
    /// Comparing the two lists is cheap; discovering it live is not.
    #[test]
    fn every_exported_primitive_is_bound_as_a_global() {
        let prelude = super::V3_PRIMITIVES_JS;
        let frozen = prelude
            .rsplit_once("Object.freeze({")
            .and_then(|(_, tail)| tail.split_once("})"))
            .map(|(inner, _)| inner)
            .expect("prelude must end by freezing its primitive object");
        let exported: std::collections::BTreeSet<&str> = frozen
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();

        let helpers = include_str!("workflow_live_v2_script_helpers.rs");
        let bound: std::collections::BTreeSet<&str> = helpers
            .lines()
            .filter_map(|line| line.trim().strip_prefix("globalThis."))
            .filter_map(|rest| rest.split_once(" = api."))
            .map(|(name, _)| name)
            .collect();

        // Guard the guard: if either parse silently yielded nothing, the
        // difference below would be empty and this test would pass vacuously.
        assert!(
            exported.contains("remediateFindings") && exported.contains("agent"),
            "failed to parse the prelude's exported primitives: {exported:?}"
        );
        assert!(
            bound.contains("agent") && bound.contains("coverageAudit"),
            "failed to parse the globals block: {bound:?}"
        );

        let missing: Vec<&str> = exported.difference(&bound).copied().collect();
        assert!(
            missing.is_empty(),
            "prelude exports {missing:?} but the globals block never binds them — an authored script calling these gets 'not defined' at dry-run pre-flight. Add `globalThis.<name> = api.<name>;` in workflow_live_v2_script_helpers.rs"
        );
    }
}

#[cfg(test)]
mod transport_retry_tests {
    /// A provider failure says nothing about the work, so it must not spend a
    /// remediation round.
    ///
    /// Observed live in one run: a 520 on a fix cost TDL-020 half its budget
    /// without a single real attempt, and a 1200s timeout on a verifier left
    /// TDL-070's ACCEPTED patch recorded as unresolved because its checker
    /// died. The second is the worse failure — correct work discarded.
    ///
    /// The host already draws this line: is_write_branch_validation_error
    /// excludes "agent transport failed" so it is not a contract violation.
    /// Asserted against the JS source because the loop is prelude text.
    #[test]
    fn transport_failures_do_not_consume_a_remediation_round() {
        let prelude = super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("const transportFailure =")
            .expect("transportFailure must exist");
        // Slice to the function's actual end, not a fixed window. A magic
        // width made this test report failure on correct code twice: the
        // instrument could not reach what it was asked to check.
        let body = &prelude[start..start + prelude[start..].find("\n    };").expect("fn end")];
        // Typed enum first, prose only where the enum cannot exist.
        assert!(
            body.contains(r#""failure_kind":"execution""#),
            "must prefer the host's typed failure kind: {body}"
        );
        assert!(
            body.contains("cancelled"),
            "a deliberate stop must not be refunded as a provider failure: {body}"
        );
        // A wholesale call failure has no branch outcomes and so no
        // failure_kind; the 520 that cost a round was exactly that shape.
        assert!(body.contains("agent transport failed"), "{body}");
        assert!(body.contains("timed out after"), "{body}");

        let loop_start = prelude
            .find("for (let round = 1; round <= maxRounds;")
            .expect("remediation loop must exist");
        let loop_body = &prelude
            [loop_start..loop_start + prelude[loop_start..].find("\n      }").expect("loop end")];
        // The round counter must advance in the BODY, not the for-header, or a
        // transport `continue` would still spend the round.
        assert!(
            !loop_body.contains("maxRounds; round += 1"),
            "round must not auto-increment: a transport retry would consume it"
        );
        assert!(loop_body.contains("round += 1"), "round must still advance");
        assert!(
            loop_body.contains("transportRetries < maxTransportRetries"),
            "transport retries must be bounded so an outage cannot spin: {loop_body}"
        );
    }
}

#[cfg(test)]
mod findings_extraction_tests {
    /// The prelude's findingsFrom must read a FANOUT envelope, not just a
    /// single-agent one. A map is a fanout: its envelope carries no
    /// data.findings, only per-branch outcomes. Reading the top level alone
    /// returned [] for every map, so reduces received nothing and the mandatory
    /// review reported clean while real findings sat unread in the branches.
    ///
    /// Asserted against the JS source because the helper is prelude text, not
    /// Rust: the shape it must traverse is `data.outcomes[i].result.data.findings`.
    #[test]
    fn findings_extraction_traverses_fanout_branch_outcomes() {
        let prelude = super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("const findingsFrom =")
            .expect("findingsFrom must exist");
        let body = &prelude[start..start + 900.min(prelude.len() - start)];
        assert!(
            body.contains("outcomes"),
            "findingsFrom must consider fanout branch outcomes: {body}"
        );
        assert!(
            body.contains("outcome.result.data.findings")
                || body.contains("outcome && outcome.result"),
            "findingsFrom must read each branch outcome's own findings: {body}"
        );
    }
}

#[cfg(test)]
mod remediation_budget_tests {
    /// Runs the REAL prelude JS, not a Rust reimplementation of it.
    ///
    /// Every other prelude guard in this file asserts on source text, which
    /// catches drift but cannot catch wrong behaviour. The budget is arithmetic
    /// over gap sets, so it is worth executing: a source assertion would have
    /// passed on a rule that funded exactly the wrong task.
    fn run_budget_js(driver: &str) -> String {
        let prelude = super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("  const remediationBudget = (opts = {}) => {")
            .expect("remediationBudget must exist");
        let end = start
            + prelude[start..]
                .find("\n  };\n")
                .expect("remediationBudget end")
            + 5;
        let script = format!("{}\n{driver}\n", &prelude[start..end]);
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("budget.mjs");
        std::fs::write(&path, script).expect("write driver");
        let out = std::process::Command::new("node")
            .arg(&path)
            .output()
            .expect("node must be available; the tests already shell out to zsh and python3");
        assert!(
            out.status.success(),
            "driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn envelope(gap_ids: &[&str]) -> String {
        let gaps = gap_ids
            .iter()
            .map(|id| format!(r#"{{"id":"{id}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"result":{{"residual_gaps":[{gaps}]}}}}"#)
    }

    /// The observed TDL-040 sequence: 5 gaps, one closed, then flat.
    /// A raw-count rule keeps funding it; this must stop on the plateau.
    #[test]
    fn budget_extends_while_the_original_diagnosis_shrinks_and_stops_on_a_plateau() {
        let a1 = envelope(&[
            "registry-missing",
            "paging",
            "mcp-path",
            "tui-alias",
            "filter-mismatch",
        ]);
        let a2 = envelope(&["paging", "mcp-path", "tui-alias", "zero-match"]);
        let a3 = envelope(&["paging", "mcp-path", "tui-alias", "zero-test-noise"]);
        let driver = format!(
            r#"const b = remediationBudget();
console.log([b.shouldContinue(1, {a1}), b.shouldContinue(2, {a2}), b.shouldContinue(3, {a3})].join(","));"#
        );
        // 1,2 are inside the base budget; 3 is the plateau and must stop.
        assert_eq!(run_budget_js(&driver), "true,true,false");
    }

    /// Churned gaps were never in the baseline, so they cannot buy budget.
    /// This is what removes the need to classify substantive vs incidental.
    #[test]
    fn gaps_absent_from_the_baseline_cannot_earn_budget() {
        let a1 = envelope(&["real-one"]);
        let a2 = envelope(&["real-one", "churn-a"]);
        let a3 = envelope(&["real-one", "churn-b"]);
        let driver = format!(
            r#"const b = remediationBudget();
b.shouldContinue(1, {a1}); b.shouldContinue(2, {a2});
console.log(b.shouldContinue(3, {a3}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// Branch-suffixed ids cannot match across attempts and would read as
    /// perpetual churn, so they are excluded rather than silently normalised.
    #[test]
    fn branch_suffixed_gap_ids_are_excluded_from_the_baseline() {
        let a1 = envelope(&["invalid_write_branch_output_review-remediate-task-tdl-020-2-63-0"]);
        let driver = format!(
            r#"const b = remediationBudget();
b.shouldContinue(1, {a1});
// Baseline is empty, so nothing can be "still closing": it must not extend
// past the base budget on the strength of an unmatchable id.
console.log(b.shouldContinue(3, {a1}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// The envelope the host produces when schema repair failed but the patch
    /// was confirmed landed against the declared baseline.
    fn schema_landed_envelope() -> String {
        r#"{"result":{"data":{"schema_repair_patch_landed":true},"residual_gaps":[]}}"#.to_string()
    }

    /// An attempt that died to schema repair WITH a landed patch produced work
    /// and no verdict, so it must not be charged to the task. Attempt 3 is
    /// refused on the flat budget; with the refund it is funded.
    #[test]
    fn a_schema_failure_with_a_landed_patch_buys_one_extra_attempt() {
        let none = envelope(&[]);
        let landed = schema_landed_envelope();
        let driver = format!(
            r#"const plain = remediationBudget();
const refunded = remediationBudget();
// Same call on both, except the refunded one also saw a landed-patch impl.
console.log([
  plain.shouldContinue(1, {none}),
  plain.shouldContinue(3, {none}),
  refunded.shouldContinue(1, {none}, {landed}),
  refunded.shouldContinue(3, {none}),
].join(","));"#
        );
        // plain: funded at 1, refused at 3. refunded: funded at 1 AND at 3.
        assert_eq!(run_budget_js(&driver), "true,false,true,true");
    }

    /// The bound is the entire safety argument. Schema repair already retries
    /// under its own cap, so an unbounded exemption turns a burned attempt into
    /// a hung task. An agent that lands a patch and emits garbage EVERY time
    /// must still run out.
    #[test]
    fn the_schema_refund_is_bounded_to_once_per_task() {
        let none = envelope(&[]);
        let landed = schema_landed_envelope();
        let driver = format!(
            r#"const b = remediationBudget();
// Four consecutive landed-patch schema failures. Only the first may pay out,
// so funding reaches base+1 = 4 and no further.
b.shouldContinue(1, {none}, {landed});
console.log([
  b.shouldContinue(3, {none}, {landed}),
  b.shouldContinue(4, {none}, {landed}),
  b.shouldContinue(5, {none}, {landed}),
].join(","));"#
        );
        // 3 < 4 funded; 4 and 5 are not — the refund did not compound.
        assert_eq!(run_budget_js(&driver), "true,false,false");
    }

    /// Without the marker nothing changes. Guards the refund against becoming a
    /// blanket extra attempt for every task — which would quietly raise the
    /// budget for the stuck tasks the progress rule exists to cut off.
    #[test]
    fn an_envelope_without_the_marker_earns_no_refund() {
        let none = envelope(&[]);
        // Shaped like the marker but false, and a look-alike key: neither pays.
        let decoys = r#"{"result":{"data":{"schema_repair_patch_landed":false,"schema_repair_patch_landed_maybe":true}}}"#;
        let driver = format!(
            r#"const b = remediationBudget();
b.shouldContinue(1, {none}, {decoys});
console.log(b.shouldContinue(3, {none}, {decoys}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// A verifier that named nothing gives nothing to measure; fall back to the
    /// flat budget rather than inventing progress from an empty set.
    #[test]
    fn an_empty_first_verdict_falls_back_to_the_flat_budget() {
        let none = envelope(&[]);
        let driver = format!(
            r#"const b = remediationBudget();
console.log([b.shouldContinue(1, {none}), b.shouldContinue(3, {none})].join(","));"#
        );
        assert_eq!(run_budget_js(&driver), "true,false");
    }
}
