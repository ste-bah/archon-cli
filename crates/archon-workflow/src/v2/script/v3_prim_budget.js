// v3_prim_budget.js: a fragment of the __archonPrimitives(w) body begun in
// v3_primitives.js (the attempt budget and the finding grouping helpers). The prelude is these files concatenated in
// order (V3_PRIMITIVES_JS); none is a module of its own.
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
  // `{ baseAttempts: 3, hardCap: 3, maxSchemaRefunds: 0 }` — which made the
  // progress check below unreachable and turned a progress-following budget
  // into a flat count of three. There is no ceiling at all now (Issue 298), so
  // `hardCap` has nothing left to narrow.
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
    // No total ceiling (Issue 298). A `hardCap` stopped a task at attempt 12
    // while it was still closing one original gap per attempt, nine open.
    // Only NO PROGRESS ends the budget; a task that closes gaps is bounded by
    // its own baseline, since every funded attempt past the window must close
    // at least one baseline gap. `hardCap` is still accepted from scripts that
    // pass it, and ignored.
    if (opts.hardCap !== undefined) {
      note(
        "remediationBudget: ignoring hardCap=" + opts.hardCap +
        "; the budget ends on no progress only, never on a total"
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
    // Floored at one so a script cannot neuter it: a generated script set
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
    let lastOpen = [];
    // Non-shrinking attempts in a row. Round 2: a plateau is TWO of them, the
    // same bound as the acceptance loop's ACCEPTANCE_STALL_LIMIT, so one noisy
    // verdict cannot end the remediation of a task that is converging.
    const PLATEAU_ATTEMPTS = 2;
    let stalls = 0;
    return {
      // Called after each attempt with the VERIFIER envelope. Returns whether
      // another attempt is warranted.
      // `implEnv` is optional: a script generated before this argument existed
      // still runs, it simply cannot earn the schema refund. Checked on BOTH
      // envelopes because one observed failure was on the write half,
      // which never reaches the verifier envelope at all.
      shouldContinue(attempt, checkEnv, implEnv) {
        const refundable = schemaRefundable(implEnv, checkEnv);
        if (schemaRefunds < maxSchemaRefunds && refundable) {
          schemaRefunds += 1;
        }
        const funded = base + schemaRefunds;
        const ids = gapIdsOf(checkEnv);
        if (baseline === null || baseline.size === 0) {
          // A verifier that named nothing gives nothing to measure against;
          // the flat window still applies. The FIRST verdict that names gaps
          // becomes the baseline, however late: closing those is progress.
          const late = baseline !== null && ids.size > 0;
          baseline = ids;
          lastRemaining = ids.size;
          lastOpen = [...ids];
          stalls = 0;
          if (attempt < funded) return true;
          // A late diagnosis has not had an attempt measured against it yet.
          if (late) return true;
          note(
            "remediationBudget: stopping after attempt " + attempt +
            ": no progress measurable (the verifier named no gap ids) within " + funded + " attempts"
          );
          return false;
        }
        // Round 2: a verdict that names no gap, or one burned by something
        // that says nothing about the work (the refund markers), measures
        // nothing. Read as "0 open" it made the next real verdict look like a
        // regression (0 -> 13) and stopped a steady closer. It is not progress
        // either, so it still counts toward the plateau: a verifier that never
        // names a gap again still runs out.
        const measured = ids.size > 0 && !refundable;
        const before = lastRemaining;
        let stillClosing = false;
        if (measured) {
          lastOpen = [...baseline].filter((id) => ids.has(id));
          // Recorded on EVERY measured call, including inside the base window.
          // Updating it only after the base attempts made a flat set look like
          // progress: the comparison fell back to the baseline size and read
          // 3 -> 3 as 5 -> 3.
          stillClosing = lastOpen.length < lastRemaining;
          lastRemaining = lastOpen.length;
        }
        stalls = stillClosing ? 0 : stalls + 1;
        if (attempt < funded) return true;
        // Extend while the ORIGINAL diagnosis is still shrinking. A plateau
        // means attempts have stopped converging, which is when more of them
        // stop being worth buying. No total caps a task that is still closing
        // gaps (Issue 298).
        if (stalls < PLATEAU_ATTEMPTS) return true;
        note(
          "remediationBudget: stopping after attempt " + attempt + ": no progress for " + stalls +
          " attempts in a row (" + before + " -> " + lastRemaining + " of " + baseline.size +
          " original gaps open: " + lastOpen.join(", ") + ")"
        );
        return false;
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
