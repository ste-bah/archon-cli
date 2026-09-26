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
    ordinal += 1;
    return await dispatchAgent(`${slug(opts.label || "agent")}-${ordinal}`, prompt, opts);
  };
  // The call `agent()` makes, under an id it was handed. Only agent() and the
  // prelude's own host-planned re-verification (Issue-111) call it; the
  // latter mints no ordinal, so no later call's id moves.
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
      });
    }
    return await w.parallel(id, items, {
      tier: opts.tier || "coder",
      // Read-only branches share the canonical checkout (one build lock).
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
  // Obs-22 (run wf-719ff3b0): the reduce was handed the map FINDINGS and
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
  const reviewMapReduce = async (label, kind, mapTask, reduceTask, acceptedTaskIds, evidenceFor) => {
    const ids = Array.isArray(acceptedTaskIds) ? acceptedTaskIds : [];
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
      reviewContract: { version: 1, kind, stage: "map", findingsPath: "data.findings", itemTaskIdsPath: "canonical_task_ids", maxFindingsPerItem: 25 },
    });
    // Attributed to the task each branch reviewed, by the host, from the
    // branch input the host built -- not from a table keyed by an item_id the
    // host never used to name branches.
    const mapFindings = reviewFindings(map);
    // Findings AND roster: which branches ran, with a zero for a clean one.
    const reduce = await w.reduce(`${label}-reduce`, { findings: mapFindings, branch_roster: reviewRoster(map) }, {
      tier: "critic",
      task: reduceTask,
      reviewContract: { version: 1, kind, stage: "reduce_final", sourceMapCallIds: [`${label}-map`], preserveMapFindings: true, findingsPath: "data.findings", accountingField: kind, maxInputBytes: 48000 },
    });
    // The host merged the map findings with the reduce's own new ones and
    // attached the result; this is the set the accounting must report.
    return reviewFindings(reduce);
  };
  const adversarialReview = async (acceptedTaskIds, opts = {}) =>
    reviewMapReduce(
      "adversarial-review",
      "adversarial_findings",
      "You did NOT do this work — be suspicious. Try to FALSIFY this accepted task using only its own claims and the bounded evidence supplied. Return data.findings as compact structured findings (max 25).",
      "The per-task findings are preserved by the host — do NOT restate them; a restated finding is dropped by identity and contributes nothing. Return data.findings for cross-task concerns ONLY: contradictions between tasks, global invariants, and PRD-level acceptance no single task owns.",
      acceptedTaskIds,
      opts.evidenceFor,
    );
  const coverageAudit = async (acceptedTaskIds, opts = {}) =>
    reviewMapReduce(
      "coverage-audit",
      "uncovered_requirements",
      "Compare this accepted task against the source requirements it claims to satisfy. Return data.findings for any requirement it appears NOT to cover.",
      "The per-task coverage findings are preserved by the host — do NOT restate them; a restated finding is dropped by identity. Return data.findings for cross-task uncovered requirements ONLY: requirements no individual task claims, and requirements two tasks each assume the other covers.",
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
  // Caller options may only WIDEN this budget, never neuter it.
  //
  // The authoring prompt shows `remediationBudget()` bare and says outright "Do
  // not replace this with a fixed bound". A generated script did exactly that —
  // `{ baseAttempts: 3, hardCap: 3, maxSchemaRefunds: 0 }` — which sets
  // `ceiling === funded`, making the progress check below unreachable and
  // turning a progress-following budget into a flat count of three.
  //
  // Measured over the run that produced it: the tasks that plateaued stopped at
  // three either way, so the cap bought nothing there; the one task whose gap
  // set was still shrinking (gaps turning over every round, severity falling to
  // none above medium) was cut at exactly three and recorded as failed. The cap
  // cost a completion and saved nothing.
  //
  // Prompt text could not prevent that, and this is the standing lesson from
  // routing: an invariant carried by prompt compliance is not an invariant. So
  // the floor is enforced here, where the script cannot reach it.
  const remediationBudget = (opts = {}) => {
    // Diagnostics only, and deliberately guarded: the prelude's own tests
    // extract this function into a standalone module where `log` is not in
    // scope. An override notice must never be able to throw.
    const note = (message) => {
      if (typeof log === "function") log(message);
    };
    const base = Math.max(1, Number(opts.baseAttempts) || 6);
    const DEFAULT_HARD_CAP = 12;
    const requestedHardCap = Number(opts.hardCap) || 0;
    const hardCap = Math.max(base, DEFAULT_HARD_CAP, requestedHardCap);
    if (requestedHardCap > 0 && requestedHardCap < hardCap) {
      note(
        "remediationBudget: ignoring hardCap=" + requestedHardCap +
        " (below the " + DEFAULT_HARD_CAP + "-attempt floor); a fixed bound disables the " +
        "progress check that funds converging tasks"
      );
    }
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
    // Floored at one for the same reason as `hardCap`: a generated script set
    // this to 0, switching the refund off entirely. Zero is not a defensible
    // choice — it charges the task for an attempt that produced work and no
    // verdict — and the bound below (once per task) is what keeps it safe, so
    // there is nothing for a caller to protect by lowering it. Raising it is
    // still permitted.
    const requestedRefunds = Number(opts.maxSchemaRefunds) || 0;
    const maxSchemaRefunds = Math.max(1, requestedRefunds);
    if (opts.maxSchemaRefunds !== undefined && requestedRefunds < maxSchemaRefunds) {
      note(
        "remediationBudget: ignoring maxSchemaRefunds=" + requestedRefunds +
        " (floored at 1); an attempt whose schema repair failed while its patch " +
        "landed produced real work and must not be charged to the task"
      );
    }
    let schemaRefunds = 0;
    // Keyed on the host's TYPED markers, never on prose. The host sets each only
    // where it can prove the condition; a bare "files changed" test would count
    // stray tool output as landed work and refund an attempt that produced none.
    //
    // Two qualifying reasons, ONE shared pool. Both are instances of the same
    // idea — an attempt burned by something that says nothing about the work:
    //
    //   schema_repair_patch_landed    schema repair failed, patch landed anyway
    //   transport_failure_no_verdict  provider/transport died before a verdict
    //
    // Separate pools would let a task that fails both ways draw two refunds,
    // quietly doubling a bound whose whole safety argument is that it is one.
    // Sharing keeps the guarantee "at most `maxSchemaRefunds` burned attempts
    // are forgiven per task" true regardless of how they were burned.
    const REFUND_MARKERS = [
      '"schema_repair_patch_landed":true',
      '"transport_failure_no_verdict":true',
    ];
    const schemaRefundable = (...envs) => {
      for (const env of envs) {
        if (!env) continue;
        let blob = "";
        try { blob = JSON.stringify(env); } catch (_) { continue; }
        if (REFUND_MARKERS.some((marker) => blob.indexOf(marker) >= 0)) return true;
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
  // The task ids a finding names, read exactly as the host's `task_ids_of`
  // reads them: the FIRST spelling that names any, trimmed. The host also
  // normalises every finding it attaches, so the two cannot disagree.
  const findingTaskIds = (finding) => {
    for (const key of ["canonical_task_ids", "task_ids", "taskIds", "task_id"]) {
      const value = finding && finding[key];
      const list = (Array.isArray(value) ? value : [value])
        .map((id) => (typeof id === "string" ? id.trim() : ""))
        .filter(Boolean);
      if (list.length > 0) return [...new Set(list)];
    }
    return [];
  };
  // The key a cross-task group is remediated under; the host's terminal rule
  // builds the same key (`cross_key`) from the same ids.
  const crossTaskKey = (ids) => `cross:${[...new Set(ids)].sort().join("+")}`;
  // A short FNV-1a hash of a whole key: `slug()` truncates at 40 characters,
  // so two long cross-task keys would otherwise share a checkpoint id.
  const keyHash = (text) => {
    let hash = 0x811c9dc5;
    for (let i = 0; i < text.length; i += 1) {
      hash ^= text.charCodeAt(i);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
    return hash.toString(16).padStart(8, "0");
  };
  // A remediation label that keeps its unit and round. `agent()` cuts a
  // label to 40 characters before the ordinal, so a long key (every
  // cross-task unit) lost its round there and two questions shared a label.
  // A label that already fits is unchanged: records filed under it replay.
  const unitLabel = (prefix, key, suffix) => {
    const plain = `${prefix}-${slug(key)}-${suffix}`;
    if (plain.length <= 40) return plain;
    const tail = `-${keyHash(key)}-${suffix}`;
    const room = Math.max(1, 40 - prefix.length - 1 - tail.length);
    return `${prefix}-${slug(key).slice(0, room).replace(/-+$/, "")}${tail}`;
  };
  const findingsByTask = (findings) => {
    const grouped = {};
    const crossTask = {};
    const unassigned = [];
    const list = Array.isArray(findings) ? findings : [];
    for (const finding of list) {
      const ids = findingTaskIds(finding);
      if (ids.length === 0) { unassigned.push(finding); continue; }
      // The host's record of a review that never completed names the task it
      // was reviewing, but no write can supply the missing verdict: a writer
      // handed it has nothing to fix, and a verifier asked whether "nothing"
      // was fixed can pass it. It stays in the accounting untouched, where the
      // host's terminal rule holds the run on it.
      if (finding && finding.review_outcome === "unreviewed") { unassigned.push(finding); continue; }
      // Ownership before ids. Reducers emit `attributable_to_task: false` when
      // no single task may act on a finding: `canonical_task_ids` then lists
      // the tasks it spans, not an owner. Routing it into each named task's
      // own remediation asked each for a change that would break the others;
      // leaving it unassigned meant nothing could ever clear it. It is
      // remediated ONCE across all of them instead: one write over the union
      // of their files, one verifier over every named task. Only the explicit
      // signal diverts — `cross_task: true` is the normal reducer case and
      // diverting on it would strand findings a task can fix alone.
      if (finding && finding.attributable_to_task === false) {
        const key = crossTaskKey(ids);
        if (!crossTask[key]) crossTask[key] = { taskIds: [...new Set(ids)].sort(), findings: [] };
        crossTask[key].findings.push(finding);
        continue;
      }
      for (const id of ids) {
        if (!grouped[id]) grouped[id] = [];
        grouped[id].push(finding);
      }
    }
    return { grouped, crossTask, unassigned };
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
    const { grouped, crossTask, unassigned } = findingsByTask([...(Array.isArray(findings) ? findings : []), ...blockedAsFindings]);
    const fileOf = (id) => (typeof opts.taskFileFor === "function" ? opts.taskFileFor(id) : "");
    const targetsOf = (id) => (typeof opts.targetFilesFor === "function" ? opts.targetFilesFor(id) : undefined);
    // One unit per task, then one per cross-task group. A group's write owns
    // the UNION of its tasks' files and its verifier judges every one of them.
    const units = [
      ...Object.keys(grouped).map((id) => ({ key: id, taskIds: [id], own: grouped[id], context: fileOf(id), targetFiles: targetsOf(id), cross: false })),
      ...Object.keys(crossTask).map((key) => {
        const group = crossTask[key];
        const union = [];
        for (const id of group.taskIds) {
          for (const file of (Array.isArray(targetsOf(id)) ? targetsOf(id) : [])) {
            if (!union.includes(file)) union.push(file);
          }
        }
        const files = group.taskIds.map(fileOf).filter(Boolean).join(", ");
        return { key, taskIds: group.taskIds, own: group.findings, context: files, targetFiles: union, cross: true };
      }),
    ];
    const maxRounds = Math.max(1, Number(opts.maxRounds) || 2);
    // The reduces whose findings this pass acts on. Naming them is what lets the
    // validator tell review-ordered remediation apart from work hidden from the
    // reviewers: it confirms these calls really are final reduces and really do
    // precede every remediation call below.
    const sourceReduceCallIds = Array.isArray(opts.sourceReduceCallIds) && opts.sourceReduceCallIds.length > 0
      ? opts.sourceReduceCallIds
      : ["adversarial-review-reduce", "coverage-audit-reduce"];
    // Issue-112b: a contest's remediation names its contest, which makes it
    // a unit of its own; absent on every other remediation.
    const contestKey = typeof opts.contestKey === "string" && opts.contestKey ? opts.contestKey : null;
    // ...and files its calls under labels of their own, so a resume's
    // lineage proofs never read a contest's answer as the review's.
    const inUnit = (suffix) => (contestKey ? `${contestKey}-${suffix}` : suffix);
    const contractFor = (stage, taskId, round, unit, esc) => Object.assign({
      version: 1,
      stage,
      taskId,
      round,
      maxRounds,
      sourceReduceCallIds,
    }, contestKey ? { contest: contestKey } : {}, unit && unit.cross ? { taskIds: unit.taskIds } : {},
    esc ? { escalation: { ownerTaskIds: esc.owners, blockerPaths: esc.files } } : {});
    // Issue-107: the HOST's cross-owner plan on a refused verdict (blocker
    // paths it mapped to other tasks through the universe), spent on ONE
    // extra round after the last regular one. Absent plan, nothing changes.
    const escalationFrom = (env, unit, targetFiles) => {
      const plan = env && env.remediation_escalation;
      const strings = (list) => (Array.isArray(list) ? list.filter((x) => typeof x === "string" && x) : []);
      const owners = strings(plan && plan.owner_task_ids);
      const files = strings(plan && plan.target_files);
      if (owners.length === 0 || files.length === 0) return null;
      let prior = "";
      try { prior = JSON.stringify({ summary: plan.refutation, blocker_evidence: plan.blocker_evidence }).slice(0, 4000); } catch (_) { prior = ""; }
      return { owners, files, prior, taskIds: [...new Set([...unit.taskIds, ...owners])], targetFiles: [...new Set([...targetFiles, ...files])] };
    };
    // Issue-111: the HOST's finding, on a fix that landed nothing after a
    // refused verdict, that the run's own later landings changed files the
    // unit or its blockers name since that verdict. It buys one read-only
    // re-verification of the tree as it is now; absent it, the refusal holds.
    const reverifyFrom = (env) => {
      const plan = env && env.remediation_reverify;
      const strings = (list) => (Array.isArray(list) ? list.filter((x) => typeof x === "string" && x) : []);
      const paths = strings(plan && plan.moved_paths);
      if (!plan || plan.source !== "host" || paths.length === 0) return null;
      if (typeof plan.fix_call_id !== "string" || typeof plan.refusal_call_id !== "string") return null;
      const stages = [...new Set((Array.isArray(plan.landings) ? plan.landings : []).map((l) => l && l.stage).filter((s) => typeof s === "string" && s))];
      return { paths, stages, fixCallId: plan.fix_call_id, refusalCallId: plan.refusal_call_id };
    };
    const resolved = [];
    const unresolved = [];
    for (const unit of units) {
      const taskId = unit.key;
      const own = unit.own;
      const verbatim = JSON.stringify(own).slice(0, 6000);
      const context = unit.context;
      const targetFiles = unit.targetFiles;
      // Every accounting entry names the unit's key; a cross-task one also
      // names the tasks it spans.
      const tag = unit.cross ? { taskId, taskIds: unit.taskIds, crossTask: true } : { taskId };
      // A remediation with nothing to write cannot be dispatched: `agent()`
      // requires at least one literal path for write work and throws otherwise,
      // which kills the whole run at the last stage. That is exactly what
      // happens to a finding no task can act on -- a PRD-level observation the
      // reviewer itself marked "not closable by a task" -- because no task
      // owns a file that would satisfy it.
      //
      // Record it honestly and move on. The finding stays visible in the
      // accounting, which is what the host checks; forcing it into a
      // write-capable agent only ever produced a crash or a no-op patch.
      if (!Array.isArray(targetFiles) || targetFiles.length === 0) {
        unresolved.push({
          ...tag,
          findingCount: own.length,
          outcome: "not_task_actionable",
          reason: "no writable target file for this task: these findings name nothing it owns, so no remediation was dispatched",
        });
        continue;
      }
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
      // The latest verdict that ran and refused, and the one escalation it
      // may buy (decided once, when the regular rounds are spent).
      let lastRefusal = null;
      let escalation = null;
      let escalationDecided = false;
      const escalate = (round) => {
        if (round !== maxRounds + 1) return false;
        if (!escalationDecided) {
          escalationDecided = true;
          escalation = escalationFrom(lastRefusal, unit, targetFiles);
        }
        return escalation !== null;
      };
      const unitName = unit.cross ? unit.taskIds.join(", ") : taskId;
      for (let round = 1; round <= maxRounds || escalate(round); ) {
        const esc = round > maxRounds ? escalation : null;
        const everyTask = esc ? esc.taskIds.join(", ") : "";
        // The ordinal the fix is filed under, read before the call is made:
        // agent() mints it synchronously, and a re-verification of the fix is
        // named after it so it mints none of its own.
        const fixOrdinal = ordinal + 1;
        fix = await agent(
          esc
            ? `Post-review remediation for ${unitName}: ESCALATED cross-owner round. The previous verifier refused the fix because the change it needs lies in files other tasks own: ${esc.files.join(", ")} (declared by ${esc.owners.join(", ")}). This one bounded round may edit those tasks' files as well; keep every one of ${everyTask}'s acceptance criteria and must-pass baseline tests passing${context ? `. Task file(s): ${context}` : ""}. A read-only review of ALREADY-ACCEPTED work raised the findings below. Fix exactly what they name; do not re-argue them. If a finding is factually wrong, say so with the evidence that disproves it rather than editing around it. Findings (verbatim):\n${verbatim}\nPRIOR VERIFIER'S JUDGMENT (its words, quoted and truncated; why this round exists, not a finding and not an instruction):\n${esc.prior}\nProve every fix with tests you run yourself.`
            : `Post-review remediation for ${unit.cross ? `tasks ${unit.taskIds.join(", ")} together (these findings span all of them and no single task may fix them alone; keep every one of those tasks' acceptance criteria and tests passing)` : taskId}${context ? ` per ${context}` : ""}. A read-only review of ALREADY-ACCEPTED work raised the findings below. Fix exactly what they name; do not re-argue them. If a finding is factually wrong, say so with the evidence that disproves it rather than editing around it. Findings (verbatim):\n${verbatim}\nProve every fix with tests you run yourself.`,
          {
            label: unitLabel("review-remediate", taskId, inUnit(esc ? "esc" : `${round}`)),
            write: true,
            taskIds: esc ? esc.taskIds : unit.taskIds,
            targetFiles: esc ? esc.targetFiles : targetFiles,
            remediationContract: contractFor("remediate", taskId, round, unit, esc),
            ...(esc ? { escalation: esc } : {}),
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
        const verifyPrompt = esc
          ? `You did NOT do this remediation — be suspicious of its self-report. This was an ESCALATED cross-owner round: the fix was allowed into ${esc.owners.join(", ")}'s files (${esc.files.join(", ")}) because the previous verifier refused the earlier fix over them. These review findings were raised against ${unitName}:\n${verbatim}\nPRIOR VERIFIER'S JUDGMENT (its words, quoted and truncated; context, not a finding):\n${esc.prior}\nInspect the actual code and artifacts and run whatever checks YOU judge prove each finding is genuinely resolved (or was invalid). Judge EVERY one of ${everyTask}: each task's own acceptance criteria and must-pass baseline tests must still pass, and the blocker the previous verifier named must be gone.`
          : `You did NOT do this remediation — be suspicious of its self-report. These review findings were raised against ${unit.cross ? unit.taskIds.join(", ") : taskId}:\n${verbatim}\nInspect the actual code and artifacts and run whatever checks YOU judge prove each finding is genuinely resolved (or was invalid).${unit.cross ? ` Judge EVERY one of ${unit.taskIds.join(", ")}: the fix spans them, so each task's own acceptance criteria and tests must still pass.` : ""}`;
        if (landedNothing(fix)) {
          log(`no patch landed for ${taskId} in round ${round}; skipping the verifier that would have run against unchanged code`);
          // Record the verify stage even though no agent runs.
          //
          // The host contract requires every `remediate` stage to be followed
          // by a `verify` for the same task, and it re-checks that against the
          // EXECUTED call sequence after the run. Skipping the verifier
          // silently left a gap the contract reads as unverified work, and a
          // completed run — implementation, both reviews and remediation all
          // accepted — was failed at the last step for it. A checkpoint states
          // the same fact the log states, in the shape the contract reads, and
          // still runs no agent against unchanged code.
          await w.checkpoint(`review-verify-${slug(taskId)}${unit.cross ? `-${keyHash(taskId)}` : ""}${contestKey ? `-${contestKey}` : ""}-${round}-no-patch`, {
            taskIds: unit.taskIds,
            remediationContract: contractFor("verify", taskId, round, unit, esc),
            summary: `no patch landed for ${taskId} in round ${round}; nothing changed to re-verify`,
          });
          check = null;
          skippedForNoPatch += 1;
          // Issue-111: "nothing changed" is the fix's claim about its own
          // round, not about the tree. When the host finds that the run's own
          // later landings changed what the refusal judged -- another task's
          // fix landed in the blocker's file meanwhile -- the refusal no
          // longer describes the tree, and one read-only verifier judges it
          // as it is now. The verdict is the round's: accepted ends the unit
          // with the no-op fix it verifies; refused stands as the latest
          // refusal. Without the host's finding the refusal holds, so an
          // unchanged tree is never re-asked and no round repeats.
          const moved = lastRefusal && acceptedEnvelope(fix) ? reverifyFrom(fix) : null;
          if (moved) {
            const reverifyId = `${slug(unitLabel("review-verify", taskId, inUnit(esc ? "esc" : `${round}`)))}-${fixOrdinal}-moved`;
            const reverifyPrompt = `${verifyPrompt}\nTHIS ROUND LANDED NO PATCH: its fix changed nothing and claims the findings are already resolved; that claim is not evidence. The last verifier refused an earlier fix, and since that verdict this run's own later landings (${moved.stages.join(", ") || "host commits"}) changed ${moved.paths.join(", ")}. Judge the repository as it is NOW: accept only if every finding is resolved on the current tree and every involved task's must-pass baseline tests pass.`;
            const reverifyOptions = {
              verify: true,
              taskIds: esc ? esc.taskIds : unit.taskIds,
              remediationContract: Object.assign(contractFor("verify", taskId, round, unit, esc), {
                reverify: { fixCallId: moved.fixCallId, refusalCallId: moved.refusalCallId },
              }),
            };
            check = await dispatchAgent(reverifyId, reverifyPrompt, reverifyOptions);
            if (acceptedEnvelope(check)) break;
            if (transportRetryable(check) && transportRetries < maxTransportRetries) {
              transportRetries += 1;
              check = await dispatchAgent(`${reverifyId}-r${transportRetries}`, reverifyPrompt, reverifyOptions);
              if (acceptedEnvelope(check)) break;
            }
            // Only an answer the host recorded is a refusal the next round
            // may be bought with: a re-verification the host refused at
            // dispatch left no record and judged nothing.
            const refusedAtDispatch = check && (check.reverify_refused || (check.data && check.data.reverify_refused));
            if (check && !refusedAtDispatch) lastRefusal = check;
            else check = null;
          }
          round += 1;
          continue;
        }
        check = await agent(
          verifyPrompt,
          {
            label: unitLabel("review-verify", taskId, inUnit(esc ? "esc" : `${round}`)),
            verify: true,
            taskIds: esc ? esc.taskIds : unit.taskIds,
            remediationContract: contractFor("verify", taskId, round, unit, esc),
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
            verifyPrompt,
            {
              label: unitLabel("review-verify", taskId, inUnit(`${esc ? "esc" : round}r${transportRetries}`)),
              verify: true,
              taskIds: esc ? esc.taskIds : unit.taskIds,
              remediationContract: contractFor("verify", taskId, round, unit, esc),
            },
          );
        }
        if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
        if (check) lastRefusal = check;
        round += 1;
      }
      // An escalated unit says so, and the round it ended on is its outcome.
      const done = escalation ? { ...tag, escalatedTo: escalation.owners } : tag;
      if (acceptedEnvelope(fix) && acceptedEnvelope(check)) {
        resolved.push({ ...done, findingCount: own.length });
      } else if (escalation && !check && lastRefusal) {
        // The escalated round landed nothing: the refusal it was bought
        // with still stands, and is what the accounting reports.
        unresolved.push({
          ...done,
          findingCount: own.length,
          outcome: "unverified",
          reason: `the escalated round landed no patch; the last verifier's refusal stands: ${summarizeEnvelope(lastRefusal)}`,
        });
      } else if (check) {
        // A verifier ran and did not accept: ordinary unfinished work.
        unresolved.push({
          ...done,
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
          ...done,
          findingCount: own.length,
          outcome: "refuted",
          reason: "the remediation agent changed nothing and asserts these findings are not valid; NOT independently verified — confirming a refutation requires asking whether the refutation is sound, not whether the code was fixed",
          refutation: verbatimEvidence(fix),
        });
      } else {
        unresolved.push({
          ...done,
          findingCount: own.length,
          outcome: "failed",
          reason: `no patch landed in ${skippedForNoPatch} of ${maxRounds} round(s); the verifier was not run because the reviewed code was never changed`,
          failure: verbatimEvidence(fix),
        });
      }
    }
    return { resolved, unresolved, unassigned };
  };
  // Result predicates the HOST owns, so a script never re-derives them.
  //
  // Every authored script has written its own `isAccepted`, `hasWorkEvidence`
  // and typed-no-op check, and guessed where a fan-out puts its outcomes. Those
  // must match host semantics exactly; when they drift the run does not fail, it
  // loops. One live run spent all six remediation rounds on a task whose work
  // was already done, because its hand-rolled predicate and the host disagreed
  // about what a no-op has to carry.
  const outcomesOf = (batch) => {
    const body = (batch && batch.data && typeof batch.data === "object") ? batch.data : batch;
    if (!body) return [];
    const outcomes = Array.isArray(body.outcomes) ? body.outcomes : null;
    const items = Array.isArray(body.items) ? body.items : null;
    if (!outcomes) {
      if (items) return items;
      return Array.isArray(batch && batch.outcomes) ? batch.outcomes : [];
    }
    if (!items) return outcomes;
    // A fanout reports each branch twice: `outcomes` carries the verdict,
    // `items` carries the evidence arrays. A branch that changed a file and ran
    // its tests can still appear in `outcomes` with both arrays empty, and a
    // caller handed that view alone concludes the branch proved nothing. That
    // is how a fully implemented task gets sent back through remediation, and
    // why returning either view on its own is wrong: only the pair describes
    // the branch. Backfill per key so an outcome that DID report evidence keeps
    // its own — the verdict is always the outcome's to state.
    const evidenceKeys = [
      "files_changed",
      "commands_run",
      "files_read",
      "artifacts",
      "evidence",
      "task_coverage",
      "residual_gaps",
    ];
    return outcomes.map((outcome, index) => {
      let item = items[index];
      const ids = (outcome && outcome.canonical_task_ids) || [];
      if (ids.length) {
        const matched = items.find((candidate) =>
          ((candidate && candidate.canonical_task_ids) || []).some((id) => ids.includes(id))
        );
        if (matched) item = matched;
      }
      const merged = Object.assign({}, item || {}, outcome || {});
      for (const key of evidenceKeys) {
        const fromOutcome = (outcome && outcome[key]) || [];
        const fromItem = (item && item[key]) || [];
        if (
          Array.isArray(fromOutcome) && fromOutcome.length === 0 &&
          Array.isArray(fromItem) && fromItem.length > 0
        ) {
          merged[key] = fromItem;
        }
      }
      return merged;
    });
  };
  const accepted = (env) => {
    const status = String((env && (env.status || (env.result && env.result.status))) || "").toLowerCase();
    return status === "accepted" || status === "noop";
  };
  // Matches the host's own rule: work evidence, or a typed no-op carrying proof.
  const usable = (env) => {
    if (!accepted(env)) return false;
    const body = (env && env.result && typeof env.result === "object" && env.result.status) ? env.result : env;
    const changed = Array.isArray(body.files_changed) && body.files_changed.length > 0;
    const ran = Array.isArray(body.commands_run) && body.commands_run.length > 0;
    if (changed || ran) return true;
    const coverage = Array.isArray(body.task_coverage) ? body.task_coverage : [];
    return coverage.some(
      (entry) =>
        entry &&
        (entry.status === "noop" || entry.status === "accepted") &&
        Array.isArray(entry.evidence) &&
        entry.evidence.some((item) => item && String(item.summary || "").trim() !== ""),
    );
  };

  // The acceptance stage (Obs-32): the task set's frozen acceptance checks,
  // run INSIDE the script as its final stage rather than by a post-terminal
  // observer that could only watch.
  //
  // A v3 run reported "complete" while the frozen acceptance-contract.json
  // checks failed: they ran after the terminal status was already committed,
  // in an observer whose authority is observe-only by contract, so the only
  // thing that could act on a failing check was a human reading a shadow
  // record. Here the checks run through the host call `acceptance-contract-run`
  // against the repository as the run left it, each failing check is routed to
  // the task(s) whose `implements` list names it through the SAME bounded
  // remediateFindings loop the reviews use, and the host re-runs the WHOLE
  // contract every round (so a fix cannot regress a passed check unseen),
  // whatever checkIds this loop names. The host records every round under
  // v2/acceptance/<round>/ and the run's terminal status is derived from the
  // last one: nothing here can mark a check passed.
  //
  // Round bookkeeping is the HOST's: it computes owning tasks, decides
  // whether a round is final (clean, last permitted, or nothing a task could
  // fix) and returns `final`; this loop only follows that verdict. The call id
  // carries the round rather than the global ordinal; the host never replays
  // an acceptance round from the store, so a resumed run re-runs each round
  // against the repository as it is now.
  let acceptanceRan = false;
  const acceptanceFailing = (env) => {
    const body = (env && env.data && typeof env.data === "object" && Array.isArray(env.data.failing)) ? env.data : env;
    return body && Array.isArray(body.failing) ? body.failing.slice() : [];
  };
  // Issue-112b: a declared path one declaring task's verified landing
  // changed, that another declarer never confirmed, holds the final gate.
  // Before acceptance, the HOST names each (path, unconfirmed declarer) on a
  // checkpoint's view; each gets ONE read-only verification of that task on
  // the tree as it is now, under the id the host planned (the host answers no
  // other). Accepted, the declarer is confirmed. Refused, the refusal goes to
  // that task's remediation (which may re-deliver the path or accept its
  // absence), and the host is asked again: whatever the other declarers now
  // owe is planned anew, each pair once per run. Nothing here decides a
  // contest; the host's contest rule reads the verdicts.
  const resolveContests = async (opts = {}) => {
    const asked = new Set();
    const outcomes = [];
    const said = (env) => String((env && (env.summary || (env.result && env.result.summary))) || "no summary").slice(0, 1200);
    for (let pass = 1; pass <= 3; pass += 1) {
      const view = await w.checkpoint(`audit-contests-${pass}`, {
        auditContests: true,
        task: "Contested declared paths: which declaring tasks the host has not seen confirm the tree as it is",
      });
      const plan = (view && (view.audit_contests || (view.data && view.data.audit_contests))) || [];
      const pending = (Array.isArray(plan) ? plan : []).filter(
        (entry) => entry && entry.source === "host" && entry.attempted !== true
          && typeof entry.confirmation_id === "string" && !asked.has(entry.confirmation_id),
      );
      if (pending.length === 0) break;
      for (const entry of pending) {
        asked.add(entry.confirmation_id);
        const file = typeof opts.taskFileFor === "function" ? opts.taskFileFor(entry.declarer) : "";
        // The facts, as the host recorded them.
        const history = entry.relanded_by
          ? `${entry.deleted_by} deleted it in its landing ${entry.deleted_in}; ${entry.relanded_by} later re-created it in its landing ${entry.relanded_in}; it currently EXISTS`
          : `${entry.deleted_by} deleted it in its landing ${entry.deleted_in}; no later landing re-created it; it currently does NOT exist`;
        const done = { taskId: entry.declarer, path: entry.path, state: entry.state };
        let refusal = null;
        if (entry.remediate === true) {
          // A refusal recorded in an earlier session whose remediation never
          // reached its end: run that remediation, never the verifier again.
          refusal = String(entry.refusal_summary || "no summary").slice(0, 1200);
        } else {
          const prompt = `Read-only verification of ${entry.declarer}${file ? ` per ${file}` : ""} against its own contract on the repository as it is NOW. The declared path ${entry.path} is contested: ${entry.declarer} declares it, and ${history}. Judge whether ${entry.declarer}'s acceptance criteria and must-pass tests hold on this tree with ${entry.path} as it is. Accept only if they do; if they need ${entry.path} otherwise, refuse and say exactly why.`;
          const check = await dispatchAgent(entry.confirmation_id, prompt, {
            verify: true,
            taskIds: [entry.declarer],
            auditContest: { path: entry.path, state: entry.state, declarer: entry.declarer },
          });
          if (accepted(check)) {
            outcomes.push({ ...done, outcome: "confirmed" });
            continue;
          }
          if (check && (check.confirmation_refused || (check.data && check.data.confirmation_refused))) {
            outcomes.push({ ...done, outcome: "not_planned", reason: said(check) });
            continue;
          }
          refusal = said(check);
        }
        const contestKey = `${entry.path}#${entry.state}#${entry.declarer}`;
        const finding = {
          id: `contested-${keyHash(contestKey)}`,
          canonical_task_ids: [entry.declarer],
          severity: "high",
          claim: `${entry.path} is contested: ${history}, and ${entry.declarer}'s own verifier refused ${entry.declarer} on that tree: ${refusal}. Make ${entry.declarer}'s contract hold: re-deliver ${entry.path} if the contract needs it, or show the contract holds without it.`,
        };
        // Its own remediation unit (`contestKey` in the contract), never a
        // round of the review's remediation of the same task.
        const remediation = await remediateFindings([finding], {
          taskFileFor: opts.taskFileFor,
          targetFilesFor: opts.targetFilesFor,
          contestKey: keyHash(contestKey),
        });
        await w.checkpoint(`${entry.confirmation_id}-done`, {
          task: `Contest remediation of ${entry.declarer} for ${entry.path} returned`,
        });
        outcomes.push({ ...done, outcome: "refused", remediation });
      }
    }
    return outcomes;
  };
  const acceptance = async (opts = {}) => {
    if (acceptanceRan) {
      throw new Error("acceptance() runs once, as the final stage after review remediation; it re-runs failing checks itself");
    }
    acceptanceRan = true;
    const contests = await resolveContests(opts);
    const maxRounds = Math.min(3, Math.max(1, Number(opts.maxRounds) || 3));
    const rounds = [];
    let checkIds = [];
    let last = null;
    for (let round = 1; round <= maxRounds; round += 1) {
      last = await w.tool(`acceptance-contract-run-${round}`, {
        tool: "acceptance-contract-run",
        round,
        maxRounds,
        checkIds,
      });
      const failing = acceptanceFailing(last);
      const entry = { round, failing_check_ids: failing.map((f) => f.check_id), remediation: null };
      rounds.push(entry);
      // The host says when the loop ends; a reply without the flag (an older
      // host) ends it too rather than looping on a shape it does not know.
      if (last.final !== false || failing.length === 0) break;
      const owned = failing.filter((f) => Array.isArray(f.owning_tasks) && f.owning_tasks.length > 0);
      if (owned.length === 0) break;
      const findings = owned.map((f) => ({
        id: `acceptance-${slug(f.check_id)}`,
        canonical_task_ids: f.owning_tasks,
        severity: "high",
        source: "acceptance-contract",
        description: `Frozen acceptance check ${f.check_id} FAILED against the finished repository: ${String(f.criterion || "").slice(0, 600)}\nkind: ${f.kind || "command"}; exit: ${f.exit_code === undefined || f.exit_code === null ? "none" : f.exit_code}${f.operational_error ? `; error: ${String(f.operational_error).slice(0, 400)}` : ""}\nstderr (tail): ${String(f.stderr_tail || "").slice(0, 1200)}\nstdout (tail): ${String(f.stdout_tail || "").slice(0, 600)}\nMake this check pass by fixing the implementation it names; do not edit the check.`,
      }));
      entry.remediation = await remediateFindings(findings, {
        maxRounds: 1,
        taskFileFor: opts.taskFileFor,
        targetFilesFor: opts.targetFilesFor,
        sourceReduceCallIds: opts.sourceReduceCallIds,
      });
      checkIds = failing.map((f) => f.check_id);
    }
    const failing = acceptanceFailing(last);
    return {
      complete: failing.length === 0 && !(last && last.operational_errors && last.operational_errors.length > 0),
      contract_present: !(last && last.contract_present === false),
      record_path: last && last.record_path,
      rounds,
      failing,
      unowned_failing: failing.filter((f) => !Array.isArray(f.owning_tasks) || f.owning_tasks.length === 0),
      passed: (last && Array.isArray(last.passed)) ? last.passed.slice() : [],
    };
  };

  return Object.freeze({ agent, agents, phase, log, pipeline, adversarialReview, coverageAudit, remediateFindings, remediationBudget, resolveContests, acceptance, accepted, usable, outcomesOf, reviewFindings, w });
}
