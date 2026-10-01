// v3_prim_complete.js: a fragment of the __archonPrimitives(w) body begun in
// v3_primitives.js (REM-14: the task set completed by the host). The prelude
// is these files concatenated in order (V3_PRIMITIVES_JS); none is a module
// of its own.
  // REM-14: a universe task the authored script never dispatched was
  // neither accepted nor blocked, and nothing implemented it. Before the
  // first review the HOST names, on a checkpoint's view, every universe task
  // no write of this session named, with the unit id it plans for it and
  // what the task declares. Each is run as a task: an implementation write,
  // a verification, retries on the progress-following budget with the
  // rejected envelopes verbatim; one it completes is reviewed with the
  // script's own tasks. Asked once per session; it mints no ordinal, so no
  // later call's id moves, and a resumed session asks the same question and
  // replays the same calls. The host's terminal rule, not this code, decides
  // each task from its records.
  let completionRun = null;
  const completionOutcome = { accepted: [], blocked: [], units: [] };
  const completionText = (env) => {
    const body = (env && env.result && typeof env.result === "object") ? env.result : env;
    if (!body) return "no envelope";
    try {
      return JSON.stringify({ status: body.status, summary: body.summary, evidence: body.evidence,
        commands_run: body.commands_run, files_changed: body.files_changed, task_coverage: body.task_coverage,
        residual_gaps: body.residual_gaps });
    } catch (_) {
      return String(body.summary || "unserializable envelope");
    }
  };
  const completionSaid = (env) => String((env && (env.summary || (env.result && env.result.summary))) || "no summary");
  const completeOne = async (entry) => {
    const t = entry.task_id;
    const list = (value) => (Array.isArray(value) ? value.filter((x) => typeof x === "string" && x) : []);
    const targetFiles = list(entry.target_files);
    const focusedTests = list(entry.focused_tests);
    const artifacts = list(entry.artifacts);
    const file = typeof entry.task_file === "string" ? entry.task_file : "";
    const per = file ? ` per its task file ${file}` : "";
    const declared = `It may write: ${targetFiles.join(", ")}.` + (artifacts.length ? ` It owns these deliverable artifacts, which its code paths must actually produce (resolve them against the project_artifact_root in YOUR OWN stage input): ${artifacts.join(", ")}.` : "")
      + (focusedTests.length ? ` Its declared focused tests: ${focusedTests.join(" ; ")}.` : "");
    const branchOf = (env) => outcomesOf(env).find((o) => Array.isArray(o && o.canonical_task_ids) && o.canonical_task_ids.includes(t)) || env;
    const write = (attempt, prompt) => dispatchAgent(`${entry.unit}-impl-${attempt}`, prompt, { write: true, taskIds: [t], targetFiles, focusedTests, artifacts });
    const verify = (attempt, prompt) => dispatchAgent(`${entry.unit}-verify-${attempt}`, prompt, { verify: true, taskIds: [t] });
    const verifyPrompt = `You did NOT implement ${t} -- be suspicious of its self-report. Re-read its contract${per}, inspect the actual code and artifacts in the repository as it is now, and run whatever tests YOU judge prove or disprove its acceptance criteria. Accept only if they hold.`;
    // A task that declares no file it may write has no write to land: ONE
    // read-only verification, which the host accepts only as a recorded
    // typed no-op (its contract holds with nothing changed).
    if (entry.mode === "noop_verify") {
      const check = await verify(1, `${verifyPrompt} ${t} declares no file it may write, so nothing was written for it. If its contract and acceptance criteria hold on the repository exactly as it is, return the typed no-op (status noop) with task_coverage evidence naming what proves each; otherwise refuse and say exactly what is missing.`);
      const status = String((check && (check.status || (check.result && check.result.status))) || "").toLowerCase();
      return settle(t, entry.unit, 1, status === "noop", check);
    }
    let impl = await write(1, `Implement ${t} exactly${per}. The host found that no write of this run implemented it, so this unit does. Read the whole task file first, then inspect the repository as it is: if the work is genuinely and verifiably done already, return the typed no-op with task_coverage evidence naming what proves it. Resolve repository paths only against the repository_root in YOUR OWN stage input. Honor the task's scope and its forbidden files. ${declared} Prove the change by running its focused tests yourself, exactly as written, and record every command.`);
    let check = await verify(1, verifyPrompt);
    const budget = remediationBudget();
    let attempt = 1;
    for (attempt = 2; budget.shouldContinue(attempt - 1, check, impl) && (!usable(branchOf(impl)) || !accepted(check)); attempt += 1) {
      const rejected = `Implementation envelope:\n${completionText(branchOf(impl))}\nVerifier envelope:\n${completionText(check)}`;
      impl = await write(attempt, `Remediate ${t}. The previous attempt was REJECTED. Fix exactly what these verbatim envelopes name; do not re-argue them:\n${rejected}\nOriginal goal: implement ${t}${per}. ${declared} Prove the fix with its focused tests, run by you, exactly as written.`);
      check = await verify(attempt, `${verifyPrompt}\nThe previous attempt was rejected with these verbatim findings:\n${rejected}`);
    }
    return settle(t, entry.unit, attempt - 1, usable(branchOf(impl)) && accepted(check), check);
  };
  const settle = (t, unit, attempts, done, check) => {
    completionOutcome.units.push({ taskId: t, unit, attempts, outcome: done ? "accepted" : "blocked" });
    if (done) completionOutcome.accepted.push(t);
    else completionOutcome.blocked.push({ taskId: t, reason: `the host's completion unit ${unit} was not accepted: ${completionSaid(check)}` });
    return done;
  };
  const runCompletion = async () => {
    const view = await w.checkpoint("task-completion", {
      taskCompletion: true,
      task: "Universe tasks no write of this session named: the host's completion units before review",
    });
    const plan = (view && (view.task_completion || (view.data && view.data.task_completion))) || [];
    const completed = [];
    for (const entry of Array.isArray(plan) ? plan : []) {
      if (!entry || entry.source !== "host" || typeof entry.task_id !== "string" || !entry.task_id || typeof entry.unit !== "string" || !entry.unit) continue;
      if (await completeOne(entry)) completed.push(entry.task_id);
    }
    return completed;
  };
  // The review's task ids: the script's own, then every task the host's
  // completion units completed. Once per session, whichever review asks.
  const completeTaskSet = async (reviewedIds) => {
    const ids = Array.isArray(reviewedIds) ? reviewedIds.slice() : [];
    if (completionRun === null) completionRun = runCompletion();
    const completed = await completionRun;
    return [...ids, ...completed.filter((id) => !ids.includes(id))];
  };
  // The blocked tasks a review remediation pass is handed: the script's,
  // then every task the completion units could not complete, so review
  // remediation works on it like any blocked task. Only the first review
  // pass (never a residual, acceptance or contest unit) takes them.
  let completionRouted = false;
  const withCompletionBlocked = (opts, blocked) => {
    const reviewPass = !(opts && (opts.residual || opts.hostEvidence === true || opts.contestKey));
    if (!reviewPass || completionRouted || completionOutcome.blocked.length === 0) return blocked;
    completionRouted = true;
    const named = (entry) => entry && (entry.taskId || entry.task_id);
    return [...blocked, ...completionOutcome.blocked.filter((entry) => !blocked.some((b) => named(b) === entry.taskId))];
  };
  // Before the acceptance stage: a task the units could not complete that
  // no review remediation pass took (the script ran none, or only other
  // kinds) gets that pass here, so it is remediated rather than only held.
  // Acceptance is the run's last stage, so this is the last point at which
  // a review pass may run.
  const routeCompletionBlocked = async () => {
    if (completionRouted || completionOutcome.blocked.length === 0) return null;
    completionOutcome.remediation = await remediateFindings([], {});
    return completionOutcome.remediation;
  };
  // The runner folds the units' outcomes into what the script returns: a
  // completed task is accepted (moved out of `blocked` if the script put it
  // there), one the units could not complete is blocked with why, unless the
  // script already accounts for it. Nothing when no unit ran.
  globalThis.__archonFoldCompletion = (result) => {
    if (completionOutcome.units.length === 0 || !result || typeof result !== "object" || Array.isArray(result)) return result;
    if (!Array.isArray(result.accepted) || !Array.isArray(result.blocked)) return result;
    const blockedId = (entry) => entry && (entry.taskId || entry.task_id);
    const folded = Object.assign({}, result);
    folded.accepted = result.accepted.slice();
    folded.blocked = result.blocked.filter((entry) => !completionOutcome.accepted.includes(blockedId(entry)));
    for (const t of completionOutcome.accepted) if (!folded.accepted.includes(t)) folded.accepted.push(t);
    for (const entry of completionOutcome.blocked) {
      if (!folded.accepted.includes(entry.taskId) && !folded.blocked.some((b) => blockedId(b) === entry.taskId)) folded.blocked.push(entry);
    }
    folded.task_completion = completionOutcome.units.slice();
    if (completionOutcome.remediation) folded.task_completion_remediation = completionOutcome.remediation;
    return folded;
  };
