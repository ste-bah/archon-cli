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
