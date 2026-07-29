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
  // Stamp the reviewed task's id onto every finding as it is COLLECTED.
  //
  // The map contract is one accepted task per item, so the primitive already
  // knows which task a finding belongs to — the branch it came out of. Leaving
  // attribution to the reviewer was measured to fail outright on run
  // wf-ee4a92fc: all 43 adversarial findings came back carrying no task key of
  // any kind (keys were claim, counter_evidence, id, severity, source, type,
  // evidence, impact, status, verdict...), so findingsByTask sent 100% of them
  // to `unassigned` and remediateFindings returned them untouched. Coverage got
  // 8 of 13 attributed only by luck of phrasing — its prompt happens to mention
  // requirement ids. The map prompt never asks for attribution at all.
  //
  // That is precisely the failure remediateFindings was written to prevent:
  // "a run could surface ~96 verified findings and exit having fixed none".
  // Fixed here rather than by adding a sentence to the prompt, because a field
  // the model is asked to remember is a field it can forget — and when it
  // forgets, the finding is silently dropped from remediation rather than
  // erroring.
  const taskIdsOfOutcome = (outcome, itemTaskIds) => {
    const declared = outcome && (outcome.canonical_task_ids || outcome.task_ids);
    if (Array.isArray(declared) && declared.length > 0) return declared;
    const itemId = outcome && outcome.item_id;
    const known = itemId ? itemTaskIds[itemId] : null;
    return known ? [known] : [];
  };
  // Never overwrite attribution the reviewer supplied itself — a finding that
  // legitimately names several tasks must keep all of them.
  const stampTaskIds = (finding, taskIds) => {
    if (!finding || typeof finding !== "object") return finding;
    const existing = finding.canonical_task_ids || finding.task_ids || finding.taskIds
      || (finding.task_id ? [finding.task_id] : null);
    if (Array.isArray(existing) && existing.length > 0) return finding;
    if (!Array.isArray(taskIds) || taskIds.length === 0) return finding;
    return Object.assign({}, finding, { canonical_task_ids: taskIds });
  };
  const attributedMapFindings = (env, itemTaskIds) => {
    const outcomes = (env && env.data && env.data.outcomes)
      || (env && env.result && env.result.data && env.result.data.outcomes);
    // Not a fanout envelope: fall back to the plain reader rather than dropping
    // findings that simply cannot be placed.
    if (!Array.isArray(outcomes)) return findingsFrom(env);
    const collected = [];
    for (const outcome of outcomes) {
      const branch = outcome && outcome.result && outcome.result.data && outcome.result.data.findings;
      if (!Array.isArray(branch)) continue;
      const taskIds = taskIdsOfOutcome(outcome, itemTaskIds);
      for (const finding of branch) collected.push(stampTaskIds(finding, taskIds));
    }
    return collected;
  };
  // The reduce is instructed to preserve map findings verbatim, but "verbatim"
  // is a model instruction, not a guarantee — the same assumption that lost the
  // ids to begin with. Re-attach attribution by identity afterwards so a
  // dropped field costs nothing.
  const findingIdentities = (finding) => {
    const keys = [];
    for (const key of ["id", "title", "claim", "summary", "finding", "requirement_id"]) {
      const value = finding && finding[key];
      if (typeof value === "string" && value.trim() !== "") {
        keys.push(`${key}:${value.trim().slice(0, 200)}`);
      }
    }
    return keys;
  };
  const reattributeFindings = (findings, stamped) => {
    const byIdentity = {};
    for (const finding of stamped) {
      const ids = finding && finding.canonical_task_ids;
      if (!Array.isArray(ids) || ids.length === 0) continue;
      for (const key of findingIdentities(finding)) {
        if (!byIdentity[key]) byIdentity[key] = ids;
      }
    }
    return (Array.isArray(findings) ? findings : []).map((finding) => {
      for (const key of findingIdentities(finding)) {
        if (byIdentity[key]) return stampTaskIds(finding, byIdentity[key]);
      }
      return finding;
    });
  };
  const reviewMapReduce = async (label, kind, mapTask, reduceTask, acceptedTaskIds, evidenceFor) => {
    const ids = Array.isArray(acceptedTaskIds) ? acceptedTaskIds : [];
    const itemTaskIds = {};
    const mapItems = ids.map((taskId) => {
      const itemId = `review-${slug(taskId)}`;
      itemTaskIds[itemId] = taskId;
      return {
        item_id: itemId,
        canonical_task_ids: [taskId],
        task: mapTask,
        evidence: (typeof evidenceFor === "function" ? evidenceFor(taskId) : []),
      };
    });
    const map = await w.parallel(`${label}-map`, mapItems, {
      tier: "critic",
      itemKind: "review_map",
      maxParallelism: 4,
      task: mapTask,
      reviewContract: { version: 1, kind, stage: "map", findingsPath: "data.findings", itemTaskIdsPath: "canonical_task_ids", maxFindingsPerItem: 25 },
    });
    const mapFindings = attributedMapFindings(map, itemTaskIds);
    const reduce = await w.reduce(`${label}-reduce`, { findings: mapFindings }, {
      tier: "critic",
      task: reduceTask,
      reviewContract: { version: 1, kind, stage: "reduce_final", sourceMapCallIds: [`${label}-map`], preserveMapFindings: true, findingsPath: "data.findings", accountingField: kind, maxInputBytes: 48000 },
    });
    return reattributeFindings(findingsFrom(reduce), mapFindings);
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
    // Carry what the agent actually SAID and SHOWED, not a rephrasing of it.
    //
    // The remediation prompt explicitly invites refutation — "If a finding is
    // factually wrong, say so with the evidence that disproves it rather than
    // editing around it." An agent that complies has produced the most valuable
    // output in the loop, and it lands in `unresolved` alongside agents that
    // tried and failed. A human triaging 56 findings cannot separate those from
    // a status, so the evidence has to travel with the record.
    const envelopeBody = (env) =>
      (env && env.result && typeof env.result === "object") ? env.result : env;
    const verbatimEvidence = (env) => {
      const body = envelopeBody(env);
      if (!body) return null;
      try {
        return JSON.stringify({
          status: body.status,
          summary: body.summary,
          evidence: body.evidence,
          commands_run: body.commands_run,
          task_coverage: body.task_coverage,
          residual_gaps: body.residual_gaps,
        }).slice(0, 8000);
      } catch (_) {
        return String((body && body.summary) || "").slice(0, 8000);
      }
    };
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
    // A half that SUCCEEDED is never a transport failure, whatever its prose
    // says.
    //
    // The markers above are substring probes over the whole serialized
    // envelope, so any agent that merely mentions a timeout — quoting a log
    // line, describing a flake it worked around, naming a test — matches. On an
    // accepted result that match is always a false positive: the work is done,
    // and the transport plainly delivered it.
    //
    // Belt to the ordering brace. Success is already evaluated before these
    // guards, so this cannot change the outcome of a completed round; it stops
    // the fix half from being re-dispatched on its own prose in the window
    // before the check has run, where there is no success pair to protect it.
    const transportRetryable = (env) => transportFailure(env) && !acceptedEnvelope(env);
    // Did the host see a patch land against the declared baseline?
    //
    // Keyed on the host's TYPED marker, set on EVERY write branch — accepted,
    // rejected and failed alike — never on prose and never on status. Status
    // cannot answer this: a wholesale size-policy rejection, an ownership
    // violation, a schema-repair exhaustion and an accepted no-op all leave the
    // reviewed code untouched while reporting four different statuses.
    //
    // Suppresses the verifier ONLY on an explicit host "nothing landed".
    //
    // The absent case must mean "run the check". Only the worktree write path
    // sets this marker, and a host predating it sets nothing at all — so
    // treating absence as "nothing landed" would silently skip EVERY verifier
    // and leave remediateFindings unable to resolve anything. Reading it that
    // way round is the difference between skipping a provably useless call and
    // disabling verification wholesale.
    const landedNothing = (env) => {
      if (!env) return false;
      let blob = "";
      try { blob = JSON.stringify(env); } catch (_) { return false; }
      // A positive marker anywhere wins: a fanout envelope can carry several
      // branches, and one that landed work is enough to make the check useful.
      if (blob.indexOf('"patch_landed":true') >= 0) return false;
      return blob.indexOf('"patch_landed":false') >= 0;
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
      // Rounds whose fix landed nothing, so the verifier was never dispatched.
      // Tracked so the unresolved reason can say WHY there is no verdict rather
      // than reporting a bare "no summary" that reads like a verifier failure.
      let skippedForNoPatch = 0;
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
        // A provider failure says nothing about the work, so it retries without
        // spending the round. Checked BEFORE the verifier is dispatched: the
        // fix half is what failed, and a verifier launched on the strength of a
        // dead fix is exactly the wasted call this loop is being taught to
        // avoid.
        if (transportRetryable(fix) && transportRetries < maxTransportRetries) {
          transportRetries += 1;
          log(`transport failure on ${taskId} remediation; retrying without consuming round ${round}`);
          continue;
        }
        // Only verify code that actually changed. This gate MUST precede the
        // verifier call — the whole defect was that it did not exist and the
        // call went out regardless.
        //
        // Observed live on TDL-041: the fix failed host validation at
        // 09:09:55.153 and the verifier started 85.8ms later against unchanged
        // code, returning the identical findings — four times across the run.
        //
        // Gated on the host's typed marker rather than on the fix's status,
        // because status answers a different question: a wholesale size-policy
        // rejection, an ownership violation and an accepted no-op all leave the
        // reviewed code exactly as the reviewers found it while reporting three
        // different statuses. A verifier pointed at code the review has already
        // examined cannot discover anything the review has not already reported.
        //
        // Nothing is forced green: with no patch there is nothing to verify, so
        // the round advances and the findings stay unresolved.
        if (landedNothing(fix)) {
          log(`no patch landed for ${taskId} in round ${round}; skipping the verifier that would have run against unchanged code`);
          check = null;
          skippedForNoPatch += 1;
          round += 1;
          continue;
        }
        check = await agent(
          `You did NOT do this remediation — be suspicious of its self-report. These review findings were raised against ${taskId}:\n${verbatim}\nInspect the actual code and artifacts and run whatever checks YOU judge prove each finding is genuinely resolved (or was invalid).`,
          {
            label: `review-verify-${slug(taskId)}-${round}`,
            verify: true,
            taskIds: [taskId],
            remediationContract: contractFor("verify", taskId, round),
          },
        );
        // SUCCESS IS TERMINAL, AND IT IS EVALUATED FIRST.
        //
        // Two accepted halves end the task. Nothing the transport classifier
        // believes can be more authoritative than the work having passed, so
        // this is checked before any guard that can `continue` or re-dispatch.
        //
        // The old order put both transport guards ahead of this break, and
        // `continue` restarts the round without ever reaching it — so an
        // accepted fix AND an accepted check were discarded unread whenever the
        // classifier matched something in the prose. That re-ran TDL-041's
        // round 2 after both halves had already passed, and cost 67 minutes
        // redoing work that was done.
        //
        // Ordering this way is what makes the guard safe independently of how
        // good the classifier is: with success settled first, a transport retry
        // can only ever add attempts to a round that genuinely failed, which is
        // all it was ever for.
        if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
        // Retrying the fix would re-apply work that may already be on disk, so
        // when only the CHECK failed, re-run the check alone.
        if (transportRetryable(check) && transportRetries < maxTransportRetries) {
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
      } else if (check) {
        // A verifier ran and did not accept: ordinary unfinished work.
        unresolved.push({
          taskId,
          findingCount: own.length,
          outcome: "unverified",
          reason: summarizeEnvelope(check),
        });
      } else if (acceptedEnvelope(fix)) {
        // The fix changed nothing and returned accepted/noop — it is ASSERTING
        // the findings are wrong. That is a different claim from "I tried and
        // could not fix this", and collapsing the two makes the refutation
        // unreadable in a list of dozens.
        //
        // Not independently verified, and deliberately not: "is this fixed?" is
        // unanswerable against an untouched tree, and a verifier sent anyway
        // resolves the ambiguity by crediting pre-existing state — which is how
        // TDL-041 got accepted on a tree nobody had modified. Confirming a
        // refutation needs a different question ("is this refutation sound?"),
        // which is answerable on unchanged code and belongs in its own pass.
        unresolved.push({
          taskId,
          findingCount: own.length,
          outcome: "refuted",
          reason: "the remediation agent changed nothing and asserts these findings are not valid; NOT independently verified — confirming a refutation requires asking whether the refutation is sound, not whether the code was fixed",
          refutation: verbatimEvidence(fix),
        });
      } else {
        unresolved.push({
          taskId,
          findingCount: own.length,
          outcome: "failed",
          reason: `no patch landed in ${skippedForNoPatch} of ${maxRounds} round(s); the verifier was not run because the reviewed code was never changed`,
          failure: verbatimEvidence(fix),
        });
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

/// Review findings must carry the task they belong to, or remediation drops them.
///
/// Executes the REAL prelude JS. A source assertion cannot catch this class of
/// bug: the code that lost the ids was syntactically fine and read correctly —
/// it simply never wrote the field, and the loss was invisible until the
/// findings reached `findingsByTask` and every one landed in `unassigned`.
/// The helpers must actually be WIRED IN, not merely correct.
///
/// The behavioural tests in the sibling modules execute the real prelude
/// helpers, but they call them directly and replay the round loop in their own
/// driver. That proves the logic and says nothing about the call sites — delete
/// every use of `attributedMapFindings` from `reviewMapReduce`, or move the
/// success break back below the transport guards, and all of them still pass.
///
/// Found by sabotage: removing the attribution call sites reddened NOTHING.
/// Correct, tested, and unreachable is this project's signature failure, and it
/// had reproduced inside the suite written to catch it. These assertions pin the
/// wiring; the behavioural tests pin the behaviour. Neither substitutes.
#[cfg(test)]
mod prelude_wiring_tests {
    fn prelude() -> &'static str {
        super::V3_PRIMITIVES_JS
    }

    fn offset_of(needle: &str) -> usize {
        prelude()
            .find(needle)
            .unwrap_or_else(|| panic!("prelude must contain `{needle}`"))
    }

    /// `reviewMapReduce` must collect findings through the attributing reader
    /// and repair the reduce output, never through the bare `findingsFrom`.
    #[test]
    fn review_map_reduce_collects_findings_through_the_attributing_reader() {
        let start = offset_of("  const reviewMapReduce = ");
        let body = &prelude()[start..start + prelude()[start..].find("\n  };").expect("fn end")];

        assert!(
            body.contains("attributedMapFindings(map, itemTaskIds)"),
            "reviewMapReduce must stamp task ids as it collects the map shards: {body}"
        );
        assert!(
            body.contains("reattributeFindings(findingsFrom(reduce)"),
            "reviewMapReduce must repair attribution the reduce dropped: {body}"
        );
        assert!(
            !body.contains("{ findings: findingsFrom(map) }"),
            "the reduce must receive STAMPED findings; passing findingsFrom(map) directly is the \
             original defect — 43 of 43 adversarial findings reached remediation unattributed"
        );
        assert!(
            body.contains("itemTaskIds[itemId] = taskId"),
            "the item_id -> taskId map must be built while the map items are constructed: {body}"
        );
    }

    /// Success must be evaluated before any guard that can `continue` or
    /// re-dispatch. Asserted on ORDER in the real loop, because the behavioural
    /// test replays the ordering in its own driver and cannot see this.
    #[test]
    fn the_success_break_precedes_the_transport_guards_in_the_real_loop() {
        let loop_start = offset_of("      for (let round = 1; round <= maxRounds;");
        let body = &prelude()[loop_start
            ..loop_start + prelude()[loop_start..].find("\n      }").expect("loop end")];

        let check_dispatch = body
            .find("label: `review-verify-${slug(taskId)}-${round}`")
            .expect("the verifier dispatch must exist");
        let success_break = body
            .find("if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;")
            .expect("the success break must exist");
        let check_transport_guard = body
            .find("transportRetryable(check)")
            .expect("the check transport guard must exist");
        let landed_gate = body
            .find("if (landedNothing(fix))")
            .expect("the landed-patch gate must exist");
        let fix_transport_guard = body
            .find("transportRetryable(fix)")
            .expect("the fix transport guard must exist");

        assert!(
            success_break < check_transport_guard,
            "two accepted halves must end the round BEFORE any transport guard: `continue` \
             restarts the round without reaching the break, which discarded an accepted pair \
             unread and re-ran TDL-041 round 2 after both halves had passed"
        );
        assert!(
            landed_gate < check_dispatch,
            "the landed-patch gate must precede the verifier dispatch, or the verifier still runs \
             against unchanged code — the defect it exists to stop"
        );
        assert!(
            fix_transport_guard < check_dispatch,
            "a dead fix must be caught before a verifier is spent on it"
        );
    }

    /// The guards must use the success-aware predicate. `transportFailure` is a
    /// substring probe over the whole envelope, so an accepted result that
    /// merely mentions a timeout matches it.
    #[test]
    fn the_round_loop_guards_use_the_success_aware_transport_predicate() {
        let loop_start = offset_of("      for (let round = 1; round <= maxRounds;");
        let body = &prelude()[loop_start
            ..loop_start + prelude()[loop_start..].find("\n      }").expect("loop end")];

        assert!(
            !body.contains("transportFailure(fix)") && !body.contains("transportFailure(check)"),
            "the loop must guard on transportRetryable, not the raw substring probe: an accepted \
             half that merely mentions a timeout in its prose is not a transport failure"
        );
        assert_eq!(
            body.matches("transportRetryable(").count(),
            2,
            "both halves must be guarded by the success-aware predicate"
        );
    }
}

/// A verifier must never be dispatched against code the fix did not change.
#[cfg(test)]
mod remediation_gate_tests {
    /// The gate's reach, pinned as an invariant instead of a claim.
    ///
    /// `patch_landed` is stamped in ONE place — `run_one_worktree_branch` — so
    /// the gate only bites on worktree writes. That is currently total coverage
    /// for its only consumer, because every write the v3 prelude can request is
    /// `write: "worktree"`, and the host does not silently downgrade: a worktree
    /// request with no workspace-boundary support is an error, never a fall back
    /// to serial or coordinated.
    ///
    /// If someone adds a coordinated or serial write to the prelude, that write
    /// gets no marker, `landedNothing` reads absence as "run the check", and the
    /// gate quietly stops applying to it — real, tested, and not reaching, which
    /// is this project's signature failure. This test is the tripwire: it fails
    /// the moment the prelude can emit a write the stamp does not cover.
    #[test]
    fn every_write_the_prelude_requests_is_a_worktree_write() {
        let prelude = super::V3_PRIMITIVES_JS;
        let modes: Vec<&str> = prelude
            .match_indices("write: \"")
            .map(|(offset, _)| {
                let rest = &prelude[offset + "write: \"".len()..];
                &rest[..rest.find('"').expect("unterminated write mode literal")]
            })
            .collect();
        assert!(
            !modes.is_empty(),
            "failed to parse any write mode from the prelude; the guard would pass vacuously"
        );
        assert!(
            modes.iter().all(|mode| *mode == "worktree"),
            "the patch_landed marker is only stamped on the worktree write path, but the prelude \
             requests {modes:?}. Either stamp the new path in workflow_live_v2_write_worktree_branch.rs's \
             sibling for that mode, or the remediation gate silently stops applying to it."
        );
    }

    fn run_gate_js(driver: &str) -> String {
        let prelude = super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("    const landedNothing = ")
            .expect("landedNothing must exist");
        let end = start + prelude[start..].find("\n    };").expect("fn end") + 7;
        let script = format!("{}\n{driver}\n", &prelude[start..end]);
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("gate.mjs");
        std::fs::write(&path, script).expect("write driver");
        let out = std::process::Command::new("node")
            .arg(&path)
            .output()
            .expect("node must be available");
        assert!(
            out.status.success(),
            "driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// One contiguous slice of the envelope helpers, so a single-expression
    /// arrow cannot be swallowed by a neighbour's terminator and re-emitted —
    /// which is exactly what a per-name extractor did here, producing a
    /// duplicate `const` that only `node` caught.
    fn envelope_helpers_js() -> String {
        let prelude = super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("    const acceptedEnvelope = ")
            .expect("acceptedEnvelope must exist");
        let end = prelude
            .find("    const blocked = ")
            .expect("blocked must follow the envelope helpers");
        assert!(start < end, "envelope helpers must precede `blocked`");
        prelude[start..end].to_string()
    }

    fn run_helpers_js(driver: &str) -> String {
        let script = format!("{}\n{driver}\n", envelope_helpers_js());
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("outcome.mjs");
        std::fs::write(&path, script).expect("write driver");
        let out = std::process::Command::new("node")
            .arg(&path)
            .output()
            .expect("node must be available");
        assert!(
            out.status.success(),
            "driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A refutation and a failure must not share a bucket, and the agent's own
    /// evidence must travel with the record.
    ///
    /// The remediation prompt invites refutation explicitly. An agent that
    /// complies produces the most valuable output in the loop and still ends in
    /// `unresolved`; with dozens of findings, a human triaging the list cannot
    /// tell it apart from an agent that tried and could not. Distinguished by a
    /// typed `outcome`, and the evidence is carried VERBATIM rather than
    /// summarised, because the summary is the part that has no triage value.
    #[test]
    fn a_refuted_finding_is_recorded_distinctly_from_a_failed_fix() {
        let driver = r#"
const classify = (fix, check) => {
  if (acceptedEnvelope(fix) && acceptedEnvelope(check)) return { outcome: "resolved" };
  if (check) return { outcome: "unverified" };
  if (acceptedEnvelope(fix)) return { outcome: "refuted", evidence: verbatimEvidence(fix) };
  return { outcome: "failed", evidence: verbatimEvidence(fix) };
};
const refuted = { status: "noop", summary: "finding F1 is wrong: registry writes ARE atomic",
  evidence: [{ kind: "proof", summary: "data_store.rs:212 write-temp-then-rename" }],
  commands_run: [{ command: "cargo test registry_atomic", status: "succeeded" }] };
const failed = { status: "failed", summary: "size policy: openbb.rs 501 > 500" };
const r = classify(refuted, null);
const f = classify(failed, null);
console.log(JSON.stringify({
  refuted: r.outcome,
  failed: f.outcome,
  keeps_refutation_text: r.evidence.indexOf("write-temp-then-rename") >= 0,
  keeps_refutation_commands: r.evidence.indexOf("registry_atomic") >= 0,
  keeps_failure_text: f.evidence.indexOf("501 > 500") >= 0,
  resolved: classify({status:"accepted"}, {status:"accepted"}).outcome,
  unverified: classify({status:"accepted"}, {status:"rejected"}).outcome
}));"#;
        assert_eq!(
            run_helpers_js(driver),
            r#"{"refuted":"refuted","failed":"failed","keeps_refutation_text":true,"keeps_refutation_commands":true,"keeps_failure_text":true,"resolved":"resolved","unverified":"unverified"}"#
        );
    }

    /// Two accepted halves must END the task, whatever the prose says.
    ///
    /// Replays the round loop in its committed order. The old order put both
    /// transport guards ahead of the success break, and `continue` restarts the
    /// round without reaching it — so an accepted fix AND an accepted check were
    /// discarded unread whenever the substring classifier matched something an
    /// agent merely mentioned. That re-ran TDL-041's round 2 after both halves
    /// had passed.
    ///
    /// Asserted on CALL SEQUENCE, not on a boolean: the defect was a wasted
    /// re-dispatch, so the only proof that matters is that the second dispatch
    /// never happens.
    #[test]
    fn two_accepted_halves_end_the_round_before_any_transport_guard_runs() {
        let driver = r#"
const run = (fixQ, checkQ) => {
  const calls = []; let round = 1, retries = 0, fix = null, check = null;
  const maxRounds = 2, maxTransportRetries = 2;
  const agent = (k) => { calls.push(k); const q = k === "fix" ? fixQ : checkQ; return q.length > 1 ? q.shift() : q[0]; };
  while (round <= maxRounds) {
    fix = agent("fix");
    if (transportRetryable(fix) && retries < maxTransportRetries) { retries += 1; continue; }
    if (landedNothing(fix)) { check = null; round += 1; continue; }
    check = agent("check");
    if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
    if (transportRetryable(check) && retries < maxTransportRetries) { retries += 1; check = agent("check"); }
    if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
    round += 1;
  }
  return calls.join(",");
};
const landed = (extra) => Object.assign({ status: "accepted", data: { patch_landed: true } }, extra);
const ok = { status: "accepted", summary: "clean" };
const dead = { status: "failed", summary: "agent transport failed: 520" };
// Both halves accepted, with transport markers sitting in ordinary agent prose.
const fixProse = landed({ summary: "fixed; the flaky suite timed out after 300s once, re-ran clean" });
const checkProse = { status: "accepted", summary: "verified; agent transport failed earlier, retried" };
console.log(JSON.stringify({
  success_is_terminal: run([fixProse], [checkProse]),
  dead_fix_retries:    run([dead, landed({})], [ok]),
  dead_check_reruns:   run([landed({})], [dead, ok]),
  persistent_dead:     run([landed({})], [dead])
}));"#;
        assert_eq!(
            run_helpers_js(driver),
            concat!(
                r#"{"success_is_terminal":"fix,check","#,
                r#""dead_fix_retries":"fix,fix,check","#,
                r#""dead_check_reruns":"fix,check,check","#,
                r#""persistent_dead":"fix,check,check,fix,check,check"}"#
            )
        );
    }

    /// The gate must be driven by the host marker, across every shape of
    /// "nothing changed" — and must stay quiet when the marker is absent.
    ///
    /// The absent case is the load-bearing one. Reading absence as "nothing
    /// landed" skips EVERY verifier: only the worktree write path sets this
    /// marker, so a host predating it, or any other write mode, would silently
    /// disable verification instead of skipping one provably useless call. That
    /// inversion was written, and caught only by executing the loop.
    #[test]
    fn the_verifier_is_suppressed_only_on_an_explicit_host_nothing_landed() {
        let driver = r#"
const cases = {
  rejected:  {"status":"failed","data":{"patch_landed":false}},
  noop:      {"status":"noop","data":{"patch_landed":false}},
  landed:    {"status":"accepted","data":{"patch_landed":true}},
  no_marker: {"status":"accepted"},
  absent_env: null,
  mixed_fanout: {"data":{"outcomes":[
    {"result":{"data":{"patch_landed":false}}},
    {"result":{"data":{"patch_landed":true}}}]}}
};
const out = {};
for (const k of Object.keys(cases)) out[k] = landedNothing(cases[k]);
console.log(JSON.stringify(out));"#;
        // Only the two explicit "false" cases suppress the verifier. A missing
        // marker, a missing envelope, and a fanout where any branch landed work
        // all keep the check running.
        assert_eq!(
            run_gate_js(driver),
            r#"{"rejected":true,"noop":true,"landed":false,"no_marker":false,"absent_env":false,"mixed_fanout":false}"#
        );
    }
}

#[cfg(test)]
mod review_attribution_tests {
    /// Pull one named arrow-function definition out of the prelude by name.
    ///
    /// Sliced to the function's real end rather than a fixed window — a magic
    /// width has already made a test in this file report failure on correct
    /// code twice.
    fn prelude_fn(name: &str) -> String {
        let prelude = super::V3_PRIMITIVES_JS;
        let marker = format!("  const {name} = ");
        let start = prelude
            .find(&marker)
            .unwrap_or_else(|| panic!("prelude must define {name}"));
        let end = start
            + prelude[start..]
                .find("\n  };")
                .unwrap_or_else(|| panic!("{name} must end with a closing arrow body"))
            + 5;
        prelude[start..end].to_string()
    }

    fn run_review_js(driver: &str) -> String {
        let mut script = String::new();
        for name in [
            "findingsFrom",
            "taskIdsOfOutcome",
            "stampTaskIds",
            "attributedMapFindings",
            "findingIdentities",
            "reattributeFindings",
            "findingsByTask",
        ] {
            script.push_str(&prelude_fn(name));
            script.push('\n');
        }
        script.push_str(driver);
        script.push('\n');
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("review.mjs");
        std::fs::write(&path, script).expect("write driver");
        let out = std::process::Command::new("node")
            .arg(&path)
            .output()
            .expect("node must be available; these tests already shell out to zsh and python3");
        assert!(
            out.status.success(),
            "driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A fanout map envelope whose findings carry NO task key of any kind —
    /// the exact shape all 43 adversarial findings had on run wf-ee4a92fc.
    const UNATTRIBUTED_MAP: &str = r#"{"data":{"outcomes":[
      {"item_id":"review-task-tdl-010","result":{"data":{"findings":[
        {"id":"F1","claim":"registry write is not atomic","severity":"high"},
        {"id":"F2","claim":"no fsync on manifest","severity":"medium"}]}}},
      {"item_id":"review-task-tdl-020","result":{"data":{"findings":[
        {"id":"F3","claim":"validation report omits gaps","severity":"high"}]}}}
    ]}}"#;

    const ITEM_TASK_IDS: &str = r#"{"review-task-tdl-010":"TASK-TDL-010","review-task-tdl-020":"TASK-TDL-020"}"#;

    /// The headline defect: without stamping, every finding routes to
    /// `unassigned` and `remediateFindings` returns them untouched.
    #[test]
    fn map_findings_are_attributed_to_the_task_whose_branch_produced_them() {
        let driver = format!(
            r#"const stamped = attributedMapFindings({UNATTRIBUTED_MAP}, {ITEM_TASK_IDS});
const {{ grouped, unassigned }} = findingsByTask(stamped);
console.log(JSON.stringify({{
  total: stamped.length,
  unassigned: unassigned.length,
  tdl010: (grouped["TASK-TDL-010"] || []).length,
  tdl020: (grouped["TASK-TDL-020"] || []).length
}}));"#
        );
        assert_eq!(
            run_review_js(&driver),
            r#"{"total":3,"unassigned":0,"tdl010":2,"tdl020":1}"#
        );
    }

    /// A reviewer that DID name its tasks must keep exactly what it named — a
    /// cross-task finding naming two tasks must not be collapsed to the one
    /// branch it happened to surface in.
    #[test]
    fn reviewer_supplied_attribution_is_never_overwritten() {
        let map = r#"{"data":{"outcomes":[{"item_id":"review-task-tdl-010","result":{"data":{"findings":[
          {"id":"F1","claim":"shared invariant broken","canonical_task_ids":["TASK-TDL-010","TASK-TDL-020"]}]}}}]}}"#;
        let driver = format!(
            r#"const stamped = attributedMapFindings({map}, {ITEM_TASK_IDS});
console.log(JSON.stringify(stamped[0].canonical_task_ids));"#
        );
        assert_eq!(run_review_js(&driver), r#"["TASK-TDL-010","TASK-TDL-020"]"#);
    }

    /// `preserveMapFindings` is an instruction to a model, not a guarantee. A
    /// reduce that returns the same findings stripped of attribution must be
    /// repaired from the stamped map set rather than silently losing routing.
    #[test]
    fn a_reduce_that_drops_attribution_is_repaired_by_identity() {
        let reduce = r#"{"data":{"findings":[
          {"id":"F1","claim":"registry write is not atomic","severity":"high"},
          {"id":"F3","claim":"validation report omits gaps","severity":"high"}]}}"#;
        let driver = format!(
            r#"const stamped = attributedMapFindings({UNATTRIBUTED_MAP}, {ITEM_TASK_IDS});
const repaired = reattributeFindings(findingsFrom({reduce}), stamped);
console.log(JSON.stringify(repaired.map((f) => f.canonical_task_ids)));"#
        );
        assert_eq!(
            run_review_js(&driver),
            r#"[["TASK-TDL-010"],["TASK-TDL-020"]]"#
        );
    }

    /// A finding the primitive genuinely cannot place must still be RETURNED.
    /// Dropping it would trade a silent routing failure for a silent data loss.
    #[test]
    fn findings_from_an_unmappable_branch_are_kept_unattributed() {
        let map = r#"{"data":{"outcomes":[{"item_id":"review-unknown-item","result":{"data":{"findings":[
          {"id":"F9","claim":"orphan finding"}]}}}]}}"#;
        let driver = format!(
            r#"const stamped = attributedMapFindings({map}, {ITEM_TASK_IDS});
const {{ unassigned }} = findingsByTask(stamped);
console.log(JSON.stringify({{ kept: stamped.length, unassigned: unassigned.length }}));"#
        );
        assert_eq!(run_review_js(&driver), r#"{"kept":1,"unassigned":1}"#);
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
