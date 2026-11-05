// v3 script dialect (marked by `export const meta`): Claude-Code-style
// primitives layered over the host API. Call ids derive deterministically
// from labels + ordinals so unchanged prefixes replay from cache.
function __archonPrimitives(w) {
  let ordinal = 0;
  // REM-10: a late review's remediation files its calls in an id space of
  // its own (`<label>-late<N>-<n>`), so no shared ordinal moves for it.
  let idSpace = null;
  const nextCallId = (label) => {
    if (idSpace) {
      idSpace.n += 1;
      return `${slug(label)}-${idSpace.tag}-${idSpace.n}`;
    }
    ordinal += 1;
    return `${slug(label)}-${ordinal}`;
  };
  // The ordinal token the next call is filed under.
  const nextOrdinalToken = () => (idSpace ? `${idSpace.tag}-${idSpace.n + 1}` : ordinal + 1);
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
  // The base-commit rule every per-task verifier carries (Obs-31). Stated
  // here in full because the item is what the verifier reads; the host's
  // `baseline_tests` stamp names the actual tests under each heading.
  const BASELINE_TEST_RULE =
    "Baseline rule: the task is NOT accepted while any test in its declared focused filter fails, " +
    "unless the host's baseline_tests section lists that test as owned by another task or as one to " +
    "leave alone. \"Pre-existing\" is not an acceptable reason to accept a red test; a pre_existing " +
    "command record is honoured only when every failing test it names is on those lists. Report every " +
    "failing test by name in matched_test_check_names.failed.";
  const agent = async (prompt, opts = {}) => {
    if (typeof prompt !== "string" || prompt.trim() === "") {
      throw new Error("agent(prompt, opts) requires a non-empty prompt string");
    }
    return await dispatchAgent(nextCallId(opts.label || "agent"), prompt, opts);
  };
  // The call `agent()` makes, under an id it was handed. Only agent() and the
  // prelude's own host-planned re-verification (Issue-111) call it; the
  // latter mints no ordinal, so no later call's id moves.
  // I5: a per-call turn bound the caller declares rides to the host, which
  // may only narrow the session's own; absent, nothing changes.
  const turnBound = (opts) => (Number.isInteger(opts && opts.maxTurns) && opts.maxTurns > 0 ? { maxTurns: opts.maxTurns } : {});
  const dispatchAgent = async (id, prompt, opts) => {
    if (opts.write) {
      assertPathList(opts.targetFiles, "agent() targetFiles", true);
      // The prompt rides on the item as `task` and nowhere else. It used to be
      // emitted twice (`task` and a byte-identical `instructions`, which no
      // host code reads), and the stage input rendered both.
      const item = {
        item_id: id,
        canonical_task_ids: opts.taskIds || [],
        task: prompt,
        target_files: opts.targetFiles || [],
        focused_verification: opts.focusedTests || [],
        artifact_requirements: opts.artifacts || [],
        work_type: "implementation",
      };
      // Issue-107: an escalated round names the owners it was widened into
      // and the exact blocker files, so the host can hold the task floor and
      // the forbidden-path lift to those files and check both against its
      // own plan at dispatch. Absent on every other write.
      if (opts.escalation) {
        item.escalation_owner_task_ids = opts.escalation.owners;
        item.escalation_blocker_paths = opts.escalation.files;
      }
      // Issue-117: a host-planned residual round names the exact files no
      // task declares that it may write; the host checks them at dispatch.
      if (Array.isArray(opts.residualFiles)) item.residual_expansion_paths = opts.residualFiles;
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
      return await w.fanout(id, [item], Object.assign(writeOptions, turnBound(opts)));
    }
    // Per-task verifiers (verify:true or focusedTests) run through the
    // HOST's verification-wave machinery: the wave id prefix grants command
    // execution and attaches every focused-verification guard (zero-test and
    // zero-command demotion, outcome normalization). commands_run is
    // agent-reported — the demotions are what keep it honest. The
    // adversarial reviewer stays a plain read-only agent by design.
    if (opts.verify === true || (Array.isArray(opts.focusedTests) && opts.focusedTests.length > 0)) {
      // Obs-31: the base-commit rule travels on the item itself, so a
      // verifier is told it even under a host that stamps no baseline. The
      // host stamps the task's actual lists (`baseline_tests`) and renders
      // them with this rule; the host also re-reads commands_run and refuses
      // an accepted verdict that leaves a non-exempt test red.
      const verifierTask = `${prompt}\n${BASELINE_TEST_RULE}`;
      const item = {
        item_id: `${id}-check`,
        canonical_task_ids: opts.taskIds || [],
        task: verifierTask,
        focused_verification: opts.focusedTests || [],
        artifact_requirements: opts.artifacts || [],
        baseline_test_rule: BASELINE_TEST_RULE,
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
        task: verifierTask,
      };
      if (opts.remediationContract) verifyOptions.remediationContract = opts.remediationContract;
      // Issue-112b: the contest a host-planned confirmation answers.
      if (opts.auditContest) verifyOptions.auditContest = opts.auditContest;
      return await w.parallel(`verification-wave-${id}`, [item], Object.assign(verifyOptions, turnBound(opts)));
    }
    return await w.agent(id, Object.assign({
      tier: opts.tier || "coder",
      task: prompt,
      targetFiles: opts.targetFiles || [],
    }, turnBound(opts)));
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
        // Not clamped for cargo: each write branch builds in its own worktree
        // with its own build cache dir, so branches never share a build lock.
        maxParallelism: opts.maxParallelism,
        task: opts.task || "Execute every item in this batch.",
        ...turnBound(opts),
      });
    }
    return await w.parallel(id, items, {
      tier: opts.tier || "coder",
      // Read-only branches share the canonical checkout (one build lock).
      maxParallelism: cargoish ? 1 : opts.maxParallelism,
      task: opts.task || "Execute every item in this batch.",
      ...turnBound(opts),
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
    const marker = w.checkpoint(nextCallId("log"), {
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
  // Review findings are COMPUTED BY THE HOST and attached to the call result
  // under `review_findings`. This reads that attachment and nothing else.
  //
  // The prelude used to walk the envelope itself -- which arrays hold
  // findings, which fan-out view to read, when two findings are the same,
  // which task a finding belongs to -- mirroring rules the host also held in
  // Rust. Six live runs failed on the two copies drifting, each after the run
  // had otherwise succeeded. There is one copy now, on the host, and the
  // accounting the host checks is compared with what the host itself handed
  // over. A reply with no attachment yields no findings: the host attaches to
  // every call that carries a reviewContract, live and in rehearsal alike.
  const reviewFindings = (env) => {
    const attached = (env && env.review_findings)
      || (env && env.data && env.data.review_findings)
      || (env && env.result && env.result.data && env.result.data.review_findings);
    return attached && Array.isArray(attached.findings) ? attached.findings.slice() : [];
  };
  const findingsFrom = (env) => reviewFindings(env);
  // Obs-22 (a live run): the reduce was handed the map FINDINGS and
  // nothing else, so two branches that reviewed their task and found nothing
  // were invisible to it, and it reported both tasks as never reviewed. A
  // findings list cannot carry an absence; a roster can. The HOST builds the
  // authoritative roster from its stored branch outcomes at dispatch and
  // replaces this one whenever it can; this copy exists so the reduce is never
  // rosterless under a host that predates it. It reads only the fan-out's
  // outcome views (item id, status, the host-stamped canonical_task_ids) and
  // counts the HOST's attributed findings per branch -- no envelope walk and
  // no finding rule of its own.
  const reviewRoster = (map) => {
    const attributed = reviewFindings(map);
    return outcomesOf(map).map((outcome) => {
      const ids = Array.isArray(outcome && outcome.canonical_task_ids) ? outcome.canonical_task_ids : [];
      const findingCount = attributed.filter((finding) =>
        Array.isArray(finding && finding.canonical_task_ids) &&
        finding.canonical_task_ids.some((id) => ids.includes(id))
      ).length;
      return {
        item_id: String((outcome && (outcome.item_id || outcome.id)) || ""),
        canonical_task_ids: ids,
        status: String((outcome && outcome.status) || ""),
        finding_count: findingCount,
      };
    });
  };
  const reviewMapReduce = async (label, kind, mapTask, reduceTask, acceptedTaskIds, evidenceFor, reduceExtra, exactIds) => {
    // REM-14: plus every universe task the host's completion units completed
    // (REM-10: a late review covers exactly the tasks it names).
    const ids = exactIds === true ? (Array.isArray(acceptedTaskIds) ? acceptedTaskIds.slice() : []) : await completeTaskSet(acceptedTaskIds);
    const mapItems = ids.map((taskId) => {
      const itemId = `review-${slug(taskId)}`;
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
      // A declared bound the plan validator reads; nothing truncates at it,
      // and the prompt asks for EVERY finding (Batch O: a reviewer told
      // "max 25" trimmed its own output).
      reviewContract: { version: 1, kind, stage: "map", findingsPath: "data.findings", itemTaskIdsPath: "canonical_task_ids", maxFindingsPerItem: 25 },
    });
    // Attributed to the task each branch reviewed, by the host, from the
    // branch input the host built -- not from a table keyed by an item_id the
    // host never used to name branches.
    const mapFindings = reviewFindings(map);
    // Findings AND roster: which branches ran, with a zero for a clean one.
    const reduce = await w.reduce(`${label}-reduce`, Object.assign({ findings: mapFindings, branch_roster: reviewRoster(map) }, reduceExtra || {}), {
      tier: "critic",
      task: reduceTask,
      reviewContract: { version: 1, kind, stage: "reduce_final", sourceMapCallIds: [`${label}-map`], preserveMapFindings: true, findingsPath: "data.findings", accountingField: kind, maxInputBytes: 48000 },
    });
    // The host merged the map findings with the reduce's own new ones and
    // attached the result; this is the set the accounting must report.
    return reviewFindings(reduce);
  };
  const ADVERSARIAL_MAP_TASK = "You did NOT do this work — be suspicious. Try to FALSIFY this accepted task using only its own claims and the bounded evidence supplied. Return data.findings as compact structured findings: EVERY finding you have, never a trimmed list.";
  const COVERAGE_MAP_TASK = "Compare this accepted task against the source requirements it claims to satisfy (the host lists them verbatim under claimed_requirements when it has them). Return data.findings for EVERY requirement it appears NOT to cover, naming the requirement id.";
  // REM-10: what each mandatory review was handed, so a task that joins the
  // reviewed set late is reviewed exactly as the others were.
  const reviewedWith = { adversarial: null, coverage: null };
  const adversarialReview = async (acceptedTaskIds, opts = {}) => {
    reviewedWith.adversarial = opts;
    return reviewMapReduce(
      "adversarial-review",
      "adversarial_findings",
      ADVERSARIAL_MAP_TASK,
      "The per-task findings are preserved by the host — do NOT restate them; a restated finding is dropped by identity and contributes nothing. Return data.findings for cross-task concerns ONLY: contradictions between tasks, global invariants, and PRD-level acceptance no single task owns.",
      acceptedTaskIds,
      opts.evidenceFor,
    );
  };
  // Batch O: the coverage audit is handed the PRD requirement inventory and
  // the frozen task set's claim map by the HOST (a checkpoint's view), so a
  // map reviewer reads the requirements its task claims verbatim and the
  // reduce can name requirements no task claims or two tasks each assume
  // the other covers. The host itself adds the requirements no task claims
  // and those only an unreviewed (blocked) task claims to the final set.
  let coverageInventorySeq = 0;
  const coverageAuditAs = async (label, kind, reduceTask, acceptedTaskIds, opts, inventoryToReduce, exactIds, inventoryId) => {
    // REM-10: a late review names its own inventory checkpoint.
    if (typeof inventoryId !== "string") coverageInventorySeq += 1;
    const view = await w.checkpoint(typeof inventoryId === "string" ? inventoryId : coverageInventorySeq === 1 ? "coverage-inventory" : `coverage-inventory-${coverageInventorySeq}`, {
      requirementInventory: true,
      task: "The PRD requirement inventory and the task set's claim map, for the coverage audit",
    });
    const found = view && (view.requirement_inventory || (view.data && view.data.requirement_inventory));
    const inventory = found && found.source === "host" ? found : null;
    const texts = {};
    for (const req of (inventory && Array.isArray(inventory.requirements) ? inventory.requirements : [])) {
      if (req && typeof req.id === "string") texts[req.id] = req.text;
    }
    const claimed = (taskId) => (inventory && inventory.by_task && Array.isArray(inventory.by_task[taskId]) ? inventory.by_task[taskId] : []);
    const evidenceFor = (taskId) => {
      const authored = typeof opts.evidenceFor === "function" ? opts.evidenceFor(taskId) : [];
      const reqs = claimed(taskId).map((id) => ({ id, text: texts[id] }));
      return reqs.length > 0 ? [...(Array.isArray(authored) ? authored : []), { claimed_requirements: reqs }] : authored;
    };
    return reviewMapReduce(label, kind, COVERAGE_MAP_TASK, reduceTask, acceptedTaskIds, evidenceFor,
      inventory && inventoryToReduce ? { requirement_inventory: inventory } : {}, exactIds);
  };
  const coverageAudit = async (acceptedTaskIds, opts = {}) => {
    reviewedWith.coverage = opts;
    return coverageAuditAs(
      "coverage-audit",
      "uncovered_requirements",
      "The per-task coverage findings are preserved by the host — do NOT restate them; a restated finding is dropped by identity. The host supplies requirement_inventory: every PRD requirement, which tasks claim each (claims), and those no task claims (unclaimed). Return data.findings for cross-task uncovered requirements ONLY: requirements no individual task claims, requirements claimed only by tasks that were not reviewed, and requirements two tasks each assume the other covers.",
      acceptedTaskIds,
      opts,
      true,
    );
  };
  // REM-10: a blocked task that review remediation finished was never in
  // the reviewed set -- the mandatory maps ran before it was done. Both maps
  // run again over exactly the tasks that moved, as their own review kinds
  // (`*_moved`: the mandatory kinds keep one map and one final reduce each),
  // and what they find is remediated like any other finding.
  let movedReviewSeq = 0;
  const LATE_TASKS = "These tasks joined the reviewed set late: each was blocked, and review remediation finished it after the mandatory reviews ran.";
  const reviewMovedTasks = async (taskIds) => {
    movedReviewSeq += 1;
    const adversarialLabel = `adversarial-review-moved-${movedReviewSeq}`;
    const coverageLabel = `coverage-audit-moved-${movedReviewSeq}`;
    const adversarial = await reviewMapReduce(adversarialLabel, "adversarial_findings_moved", ADVERSARIAL_MAP_TASK,
      `${LATE_TASKS} The per-task findings are preserved by the host — do NOT restate them; a restated finding is dropped by identity and contributes nothing. Return data.findings for cross-task concerns among these tasks ONLY: contradictions between them and invariants they break together.`,
      taskIds, (reviewedWith.adversarial || {}).evidenceFor, {}, true);
    const uncovered = await coverageAuditAs(coverageLabel, "uncovered_requirements_moved",
      `${LATE_TASKS} The per-task coverage findings are preserved by the host — do NOT restate them; a restated finding is dropped by identity. Return data.findings ONLY for requirements these tasks each assume another of them covers.`,
      taskIds, reviewedWith.coverage || {}, false, true, `coverage-inventory-moved-${movedReviewSeq}`);
    return { seq: movedReviewSeq, taskIds, adversarial, uncovered, reduceCallIds: [`${adversarialLabel}-reduce`, `${coverageLabel}-reduce`] };
  };
