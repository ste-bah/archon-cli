// v3_prim_remediate.js: a fragment of the __archonPrimitives(w) body begun
// in v3_primitives.js (remediateFindings). The prelude is these files
// concatenated in order (V3_PRIMITIVES_JS); none is a module of its own.
  // Act on review findings instead of only reporting them, and never count a
  // finding fixed that no verifier judged fixed.
  //
  // Batch O. Under a host plan (`requestRemediationPlan`) a review pass is
  // judged PER FINDING: each carries the host's id, each round's verifier
  // returns a disposition for every id it was given, and the host's reading
  // of it (`hostReading`) closes the ids it proves. The ids left open go to
  // the next round with the verifier's reasons; a cycle of rounds that closed
  // any id buys another cycle over what is left, so the budget follows
  // progress, and a cycle that revisits an unresolved state pauses the run. Whatever is
  // still open then is one unresolved entry per id, which holds the run. A
  // fix that changed nothing and asserts its findings are invalid is put to a
  // read-only verifier asking whether that refutation is sound. Findings no
  // task names are routed by the host, at any severity; nothing is dropped.
  //
  // Host-planned residual rounds, acceptance-stage units and contest units
  // keep their own one-verdict rules: the host's gates judge those.
  const remediateFindings = async (findings, opts = {}) => {
    // REM-10: the pass this prelude runs over a late review's findings,
    // marked by a key no script can name.
    const late = opts[LATE_PASS] || null;
    // REM-14: plus the tasks the host's completion units could not complete.
    const blocked = late ? [] : withCompletionBlocked(opts, Array.isArray(opts.blockedTasks) ? opts.blockedTasks : []);
    const blockedAsFindings = blocked
      .filter((entry) => entry && entry.taskId)
      .map((entry) => ({
        canonical_task_ids: [entry.taskId],
        id: `blocked-task-${slug(entry.taskId)}`,
        blocked_task: entry.taskId,
        description: `This task exhausted its remediation budget without passing verification. Last verifier summary: ${String(entry.reason || "no summary")}`,
      }));
    const all = [...(Array.isArray(findings) ? findings : []), ...blockedAsFindings];
    const contestKey = typeof opts.contestKey === "string" && opts.contestKey ? opts.contestKey : null;
    const residual = opts.residual && typeof opts.residual.key === "string"
      ? { key: opts.residual.key, files: stringList(opts.residual.files),
        verifyNote: typeof opts.residual.verifyNote === "string" ? opts.residual.verifyNote : "",
        pass: Number.isInteger(opts.residual.pass) && opts.residual.pass >= 2 ? opts.residual.pass : 1 }
      : null;
    const hostEvidence = opts.hostEvidence === true;
    const reviewPass = !residual && !hostEvidence && !contestKey;
    const plan = reviewPass && all.length > 0 ? await requestRemediationPlan(all, late ? `remediation-plan-moved-${late.seq}` : undefined) : null;
    const perId = plan !== null;
    const { units, unassigned, checks } = planUnits(all, plan, opts, !residual && !hostEvidence);
    if (residual) for (const unit of units) unit.targetFiles = [...new Set([...(Array.isArray(unit.targetFiles) ? unit.targetFiles : []), ...residual.files])];
    const maxRounds = Math.max(1, Number(opts.maxRounds) || 2);
    const sourceReduceCallIds = Array.isArray(opts.sourceReduceCallIds) && opts.sourceReduceCallIds.length > 0
      ? opts.sourceReduceCallIds
      : ["adversarial-review-reduce", "coverage-audit-reduce"];
    const observedBy = stringList(opts.observedBy);
    const inUnit = (suffix) => (contestKey ? `${contestKey}-${suffix}` : suffix);
    const frozenChecks = hostEvidence && opts.frozenChecks && typeof opts.frozenChecks === "object" ? opts.frozenChecks : {};
    const ctx = { maxRounds, sourceReduceCallIds, observedBy, contestKey, residual, perId, checks, frozenChecks, inUnit };
    const resolved = [];
    const unresolved = [];
    let residualRefused = false;
    // Units whose cycle revisited an unresolved state. The pause waits until
    // every unit of the pass had its chance: one unit's stall never stops
    // another unit that can still make progress.
    const stalls = [];
    for (const unit of units) {
      const tag = unit.cross ? { taskId: unit.key, taskIds: unit.taskIds, crossTask: true } : { taskId: unit.key };
      // A unit with nothing it may write cannot be dispatched (`agent()`
      // requires a literal path). Under a host plan that cannot happen for a
      // finding that names a file or a task with files; what remains is
      // recorded open per finding, and holds the run.
      if (!Array.isArray(unit.targetFiles) || unit.targetFiles.length === 0) {
        if (perId) {
          for (const f of unit.own) unresolved.push({ ...tag, findingId: f.finding_id, findingCount: 1, outcome: "no_writable_target", reason: "the host found no file this finding's tasks may write or be granted" });
        } else {
          unresolved.push({ ...tag, findingCount: unit.own.length, outcome: "not_task_actionable", reason: "no writable target file for this task: these findings name nothing it owns, so no remediation was dispatched" });
        }
        continue;
      }
      let open = unit.own.slice();
      const closedIds = [];
      let last = null;
      const state = () => JSON.stringify(open.map((f) => f.finding_id).sort());
      const seen = new Set([state()]);
      for (let cycle = 1; open.length > 0; cycle += 1) {
        last = await remediationCycle(unit, open, cycle, ctx, last ? last.reasons : null);
        if (last.residualRefused) residualRefused = true;
        const closed = new Set(last.closed);
        closedIds.push(...open.filter((f) => closed.has(f.finding_id)).map((f) => f.finding_id));
        open = open.filter((f) => !closed.has(f.finding_id));
        if (!perId) break;
        const key = state();
        if (open.length > 0 && seen.has(key)) {
          stalls.push({ unit: unit.key, part: unit.part || 0, cycle, failing_ids: JSON.parse(key),
            task_ids: unit.taskIds || [unit.key], reasons: last.reasons, evidence: { fix: last.fix, check: last.check } });
          break;
        }
        seen.add(key);
      }
      const done = last && last.escalation ? { ...tag, escalatedTo: last.escalation.owners } : tag;
      if (perId) {
        if (closedIds.length > 0) resolved.push({ ...done, findingIds: closedIds, findingCount: closedIds.length });
        for (const f of open) {
          unresolved.push({
            ...done,
            findingId: f.finding_id,
            findingCount: 1,
            outcome: last.verified ? "unverified" : "failed",
            reason: last.reasons[f.finding_id] || (last.verified ? "no verifier closed it" : `no patch landed in ${last.skippedForNoPatch} round(s) and no verifier judged it`),
            ...(last.check ? { evidence: verbatimEvidence(last.check) } : last.fix ? { failure: verbatimEvidence(last.fix) } : {}),
          });
        }
        continue;
      }
      legacyOutcome(last, done, unit, maxRounds, resolved, unresolved);
    }
    if (stalls.length > 0) {
      // A bounded id: a digest (FNV-1a) of the sorted stalled units; the
      // units themselves travel in the evidence.
      const units = stalls.map((s) => `${s.unit}#${s.part}@${s.cycle}`).sort().join("|");
      let digest = 0x811c9dc5;
      for (let i = 0; i < units.length; i += 1) digest = Math.imul(digest ^ units.charCodeAt(i), 0x01000193) >>> 0;
      await w.checkpoint(inUnit(`remediation-stall-${stalls.length}-${digest.toString(16).padStart(8, "0")}`), {
        remediationPause: { cause: "no_progress", failing_ids: [...new Set(stalls.flatMap((s) => s.failing_ids))].sort(),
          task_ids: [...new Set(stalls.flatMap((s) => s.task_ids))].sort(), stalls },
      });
      throw new Error("the host returned from a remediation stall without pausing");
    }
    // REM-10: a blocked task this pass finished (every finding standing for
    // it closed) was never reviewed. Both maps run over exactly those tasks,
    // and what they find is remediated in a pass of its own, whose units
    // name the late reviews as their source.
    // Each task is reviewed late once per session, whichever pass moved it.
    const moved = perId && reviewPass && !late ? movedTasks(blocked, units, resolved).filter((t) => !lateReviewed.has(t)) : [];
    if (moved.length === 0) return Object.assign({ resolved, unresolved, unassigned }, residual ? { residualRefused } : {});
    for (const t of moved) lateReviewed.add(t);
    const review = await reviewMovedTasks(moved);
    const found = [...review.adversarial, ...review.uncovered];
    let again = { resolved: [], unresolved: [], unassigned: [] };
    if (found.length > 0) {
      // Its own id space: no shared ordinal or checkpoint counter moves.
      const outer = idSpace;
      idSpace = { tag: `late${review.seq}`, n: 0 };
      try {
        again = await remediateFindings(found, Object.assign({}, opts, { blockedTasks: [], sourceReduceCallIds: review.reduceCallIds,
          observedBy: review.reduceCallIds, [LATE_PASS]: { seq: review.seq } }));
      } finally {
        idSpace = outer;
      }
    }
    return {
      resolved: [...resolved, ...again.resolved],
      unresolved: [...unresolved, ...again.unresolved],
      unassigned: [...unassigned, ...again.unassigned],
      movedReview: { taskIds: moved, adversarial_findings: review.adversarial, uncovered_requirements: review.uncovered },
    };
  };
  const LATE_PASS = Symbol("latePass");
  const lateReviewed = new Set();
  // The blocked tasks whose every standing finding a verifier closed, in the
  // order the script listed them.
  const movedTasks = (blocked, units, resolved) => {
    const closed = new Set(resolved.flatMap((entry) => stringList(entry && entry.findingIds)));
    const standing = {};
    for (const unit of units) {
      for (const f of unit.own) {
        if (f && typeof f.blocked_task === "string") (standing[f.blocked_task] = standing[f.blocked_task] || []).push(f.finding_id);
      }
    }
    const tasks = [...new Set(blocked.map((entry) => entry && entry.taskId).filter((id) => typeof id === "string" && id))];
    return tasks.filter((id) => Array.isArray(standing[id]) && standing[id].every((fid) => closed.has(fid)));
  };
  // How a unit judged by one verdict ends (residual, acceptance, contest,
  // or a host that planned nothing).
  const legacyOutcome = (last, done, unit, maxRounds, resolved, unresolved) => {
    const own = unit.own;
    const { fix, check, lastRefusal, escalation, skippedForNoPatch } = last;
    if (acceptedEnvelope(fix) && acceptedEnvelope(check)) {
      resolved.push({ ...done, findingCount: own.length });
    } else if (escalation && !check && lastRefusal) {
      unresolved.push({ ...done, findingCount: own.length, outcome: "unverified",
        reason: `the escalated round landed no patch; the last verifier's refusal stands: ${summarizeEnvelope(lastRefusal)}` });
    } else if (check) {
      unresolved.push({ ...done, findingCount: own.length, outcome: "unverified", reason: summarizeEnvelope(check) });
    } else if (acceptedEnvelope(fix)) {
      unresolved.push({ ...done, findingCount: own.length, outcome: "refuted",
        reason: "the remediation agent changed nothing and asserts these findings are not valid; NOT independently verified",
        refutation: verbatimEvidence(fix) });
    } else {
      unresolved.push({ ...done, findingCount: own.length, outcome: "failed",
        reason: `no patch landed in ${skippedForNoPatch} of ${maxRounds} round(s); the verifier was not run because the reviewed code was never changed`,
        failure: verbatimEvidence(fix) });
    }
  };
