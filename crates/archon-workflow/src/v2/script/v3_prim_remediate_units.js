// v3_prim_remediate_units.js: a fragment of the __archonPrimitives(w) body
// begun in v3_primitives.js (the remediation envelope readers, the host's
// remediation plan, and how findings become units). The prelude is these
// files concatenated in order (V3_PRIMITIVES_JS); none is a module of its own.
  // Envelope readers for remediation rounds. The author's own isAccepted and
  // summarize live in the authored script, not here, so the prelude never
  // depends on them.
  const acceptedEnvelope = (env) => {
    const status = String((env && (env.status || (env.result && env.result.status))) || "").toLowerCase();
    return ["accepted", "passed", "ok", "succeeded", "success", "complete", "completed", "verified_noop", "noop"].indexOf(status) >= 0;
  };
  // Accounting text only (a reason an operator reads), never a decision;
  // marked when it is shortened.
  const clipMarked = (text, limit) => (text.length > limit ? `${text.slice(0, limit)} [truncated; ${text.length} chars]` : text);
  const summarizeEnvelope = (env) =>
    clipMarked(String((env && (env.summary || (env.result && env.result.summary))) || "no summary"), 2000);
  // Carry what the agent actually SAID and SHOWED, not a rephrasing of it: a
  // refutation is the most valuable output of the loop, and a human triaging
  // dozens of open findings cannot separate it from a failure by status.
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
      });
    } catch (_) {
      return String((body && body.summary) || "");
    }
  };
  // A call the PROVIDER failed says nothing about the work. Typed signal
  // first (BranchFailureKind::Execution), then the exact markers the host
  // itself excludes from write-branch validation errors; a cancellation is a
  // deliberate stop, never a provider failure.
  const transportFailure = (env) => {
    if (!env) return false;
    let blob = "";
    try { blob = JSON.stringify(env); } catch (_) { return false; }
    if (blob.indexOf("cancelled") >= 0) return false;
    if (blob.indexOf('"failure_kind":"execution"') >= 0) return true;
    return blob.indexOf("agent transport failed") >= 0
      || blob.indexOf("timed out after") >= 0;
  };
  // A half that SUCCEEDED is never a transport failure, whatever its prose.
  const transportRetryable = (env) => transportFailure(env) && !acceptedEnvelope(env);
  // The host's typed "nothing landed" marker, set on every write branch.
  // Absent means "run the check": a host predating the marker sets none, and
  // reading absence as "nothing landed" would skip every verifier.
  const landedNothing = (env) => {
    if (!env) return false;
    let blob = "";
    try { blob = JSON.stringify(env); } catch (_) { return false; }
    if (blob.indexOf('"patch_landed":true') >= 0) return false;
    return blob.indexOf('"patch_landed":false') >= 0;
  };
  // The host's typed marker that a branch's patch landed.
  const landedSomething = (env) => {
    if (!env) return false;
    try { return JSON.stringify(env).indexOf('"patch_landed":true') >= 0; } catch (_) { return false; }
  };
  const stringList = (list) => (Array.isArray(list) ? list.filter((x) => typeof x === "string" && x) : []);
  // Batch O: the host's plan for a review remediation pass (ids, owners,
  // grants, which findings concern a check, and every planned task's declared
  // files). One checkpoint per pass, numbered in script order; it mints no
  // ordinal, so no later call's id moves. `null` when the host gave none.
  let remediationPlanSeq = 0;
  const requestRemediationPlan = async (findings, explicitId) => {
    // REM-10: a late review's pass names its own plan checkpoint.
    if (typeof explicitId !== "string") remediationPlanSeq += 1;
    const view = await w.checkpoint(typeof explicitId === "string" ? explicitId : `remediation-plan-${remediationPlanSeq}`, {
      remediationPlan: true,
      findings,
      task: `Remediation plan: the host's id, owners and grants for each of ${findings.length} finding(s)`,
    });
    const plan = view && (view.remediation_plan || (view.data && view.data.remediation_plan));
    if (!plan || plan.source !== "host" || !Array.isArray(plan.findings) || plan.findings.length !== findings.length) return null;
    return plan;
  };
  // The host's per-finding reading of a remediation verifier (Batch O).
  const hostReading = (env) => {
    const read = env && (env.remediation_dispositions || (env.data && env.data.remediation_dispositions));
    if (!read || read.source !== "host") return { closed: [], open: {} };
    return { closed: stringList(read.closed), open: read.open && typeof read.open === "object" ? read.open : {} };
  };
  // Findings are never cut. A unit whose findings do not fit one prompt is
  // split into several units of WHOLE findings; a single finding larger than
  // the budget is a unit of its own, whole.
  const REMEDIATION_UNIT_CHARS = 8000;
  const chunkFindings = (own) => {
    const chunks = [];
    let current = [];
    let size = 0;
    for (const finding of own) {
      const length = JSON.stringify(finding).length;
      if (current.length > 0 && size + length > REMEDIATION_UNIT_CHARS) {
        chunks.push(current);
        current = [];
        size = 0;
      }
      current.push(finding);
      size += length;
    }
    if (current.length > 0) chunks.push(current);
    return chunks;
  };
  // The units a pass remediates: one per task, one per cross-task group,
  // each split by `chunkFindings`. With a host plan, who fixes each finding
  // and what it may write are the host's; without one, the findings' own
  // task ids decide, as before. Every finding carries its id.
  const planUnits = (all, plan, opts, split) => {
    const fileOf = (id) => (typeof opts.taskFileFor === "function" ? opts.taskFileFor(id) : "");
    const targetsOf = (id) => {
      const authored = typeof opts.targetFilesFor === "function" ? opts.targetFilesFor(id) : undefined;
      const scope = plan && plan.task_scope && Array.isArray(plan.task_scope[id]) ? stringList(plan.task_scope[id]) : [];
      if (scope.length === 0) return authored;
      return [...new Set([...(Array.isArray(authored) ? authored : []), ...scope])];
    };
    // Batch E: files the host granted a finding join its unit's targets --
    // read only from the host (the acceptance loop's own option, keyed by the
    // ids it built from the host's reply, or the host's plan), never from a
    // finding's fields, which a reducer's agent output could carry.
    const hostGrants = Object.assign({}, opts.hostGrants && typeof opts.hostGrants === "object" ? opts.hostGrants : {});
    const checks = new Set();
    const grouped = {};
    const crossTask = {};
    const unassigned = [];
    all.forEach((finding, index) => {
      const entry = plan ? plan.findings[index] : null;
      // Without a host plan, an id the finding already carries (one this
      // prelude stamped on an earlier pass) is kept, so a recorded finding
      // re-sent on a resume keeps its prompt; the host never reads it.
      const carried = finding && typeof finding.finding_id === "string" && finding.finding_id ? finding.finding_id : null;
      const id = entry && typeof entry.finding_id === "string" && entry.finding_id
        ? entry.finding_id
        : carried || `js-${keyHash(JSON.stringify(finding))}`;
      const stamped = Object.assign({}, finding, { finding_id: id });
      if (entry) {
        if (stringList(entry.grants).length > 0) hostGrants[id] = stringList(entry.grants);
        if (entry.check === true) checks.add(id);
      }
      // The host's record of a review that never completed: no write can
      // supply the missing verdict, so it stays in the accounting untouched,
      // where the host's terminal rule holds the run on it.
      if (finding && finding.review_outcome === "unreviewed") { unassigned.push(finding); return; }
      const placed = Boolean(plan && plan.placed === true && entry);
      const ids = placed ? stringList(entry.task_ids) : findingTaskIds(finding);
      const cross = placed ? entry.cross === true : Boolean(finding && finding.attributable_to_task === false);
      if (ids.length === 0) { unassigned.push(finding); return; }
      if (cross) {
        const key = crossTaskKey(ids);
        if (!crossTask[key]) crossTask[key] = { taskIds: [...new Set(ids)].sort(), findings: [] };
        crossTask[key].findings.push(stamped);
        return;
      }
      for (const taskId of ids) {
        if (!grouped[taskId]) grouped[taskId] = [];
        grouped[taskId].push(stamped);
      }
    });
    const withGrants = (targets, own) => {
      const granted = [];
      for (const f of own) {
        const key = f && typeof f.id === "string" && Object.prototype.hasOwnProperty.call(hostGrants, f.id) ? f.id : f && f.finding_id;
        const files = Object.prototype.hasOwnProperty.call(hostGrants, key) ? hostGrants[key] : [];
        for (const p of stringList(files)) if (!granted.includes(p)) granted.push(p);
      }
      return granted.length === 0 ? targets : [...new Set([...(Array.isArray(targets) ? targets : []), ...granted])];
    };
    const whole = [
      ...Object.keys(grouped).map((id) => ({ key: id, taskIds: [id], own: grouped[id], context: fileOf(id), targets: targetsOf(id), cross: false })),
      ...Object.keys(crossTask).map((key) => {
        const group = crossTask[key];
        const union = [];
        for (const id of group.taskIds) {
          for (const file of (Array.isArray(targetsOf(id)) ? targetsOf(id) : [])) if (!union.includes(file)) union.push(file);
        }
        return { key, taskIds: group.taskIds, own: group.findings, context: group.taskIds.map(fileOf).filter(Boolean).join(", "), targets: union, cross: true };
      }),
    ];
    const units = [];
    for (const unit of whole) {
      const chunks = split ? chunkFindings(unit.own) : [unit.own];
      chunks.forEach((own, index) => {
        units.push({
          key: unit.key,
          taskIds: unit.taskIds,
          own,
          context: unit.context,
          targetFiles: withGrants(unit.targets, own),
          cross: unit.cross,
          part: chunks.length > 1 ? `${index + 1}of${chunks.length}` : "",
        });
      });
    }
    return { units, unassigned, checks };
  };
