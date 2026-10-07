// v3_prim_accept.js: a fragment of the __archonPrimitives(w) body begun in
// v3_primitives.js (host-owned result predicates, contests, residual rounds and the acceptance stage). The prelude is these files concatenated in
// order (V3_PRIMITIVES_JS); none is a module of its own.
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
  // Issue-122: a host-planned unit a resumed session skips, or re-enters,
  // takes its calls' place in the ordinal from the host's own records
  // (`resume_ordinals`), so every later call keeps the id it was filed
  // under. Never below where the unit loop began: no call can take an
  // earlier call's id. A unit the host has no record of moves nothing.
  const ordinalAligner = () => {
    const start = ordinal;
    let high = ordinal;
    const valid = (n) => Number.isInteger(n) && n >= 0;
    return {
      // A unit an earlier session finished: where it left the ordinal, never
      // below any unit this session has run.
      to: (n) => { if (valid(n)) ordinal = Math.max(high, n); },
      // A unit re-entered here: its first fix where it was filed, never
      // below where the loop began (its own labels keep it apart from any
      // unit this session ran).
      refile: (n) => { if (valid(n)) ordinal = Math.max(start, n); },
      // After a unit this session RAN: nothing later reaches back into it.
      raise: () => { high = Math.max(high, ordinal); ordinal = high; },
    };
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
    // Never cut: a refusal's words are what its remediation acts on.
    const said = (env) => String((env && (env.summary || (env.result && env.result.summary))) || "no summary");
    const align = ordinalAligner();
    // Batch O: until the host plans nothing new (each pair is asked once per
    // run, so this ends), never a fixed count of passes.
    for (let pass = 1; ; pass += 1) {
      const view = await w.checkpoint(`audit-contests-${pass}`, {
        auditContests: true,
        task: "Contested declared paths: which declaring tasks the host has not seen confirm the tree as it is",
      });
      const plan = (view && (view.audit_contests || (view.data && view.data.audit_contests))) || [];
      const listed = Array.isArray(plan) ? plan : [];
      const pending = new Set(listed.filter(
        (entry) => entry && entry.source === "host" && entry.attempted !== true
          && typeof entry.confirmation_id === "string" && !asked.has(entry.confirmation_id),
      ));
      // Issue-122: a pair an earlier session finished makes no call here;
      // its calls' place in the ordinal is taken as the host recorded it.
      const skipped = (entry) => {
        if (entry && entry.source === "host" && entry.attempted === true
          && typeof entry.confirmation_id === "string" && !asked.has(entry.confirmation_id)) align.to(entry.resume_ordinal);
      };
      if (pending.size === 0) { listed.forEach(skipped); break; }
      let ran = false;
      for (const entry of listed) {
        if (ran) { align.raise(); ran = false; }
        if (!pending.has(entry)) { skipped(entry); continue; }
        asked.add(entry.confirmation_id);
        ran = true;
        const file = typeof opts.taskFileFor === "function" ? opts.taskFileFor(entry.declarer) : "";
        // The facts, as the host recorded them.
        const history = entry.relanded_by
          ? `${entry.deleted_by} deleted it in its landing ${entry.deleted_in}; ${entry.relanded_by} later re-created it in its landing ${entry.relanded_in}; it currently EXISTS`
          : `${entry.deleted_by} deleted it in its landing ${entry.deleted_in}; no later landing re-created it; it currently does NOT exist`;
        const done = { taskId: entry.declarer, path: entry.path, state: entry.state };
        let refusal = null;
        let recorded = null;
        if (entry.remediate === true) {
          // A refusal recorded in an earlier session whose remediation never
          // reached its end: run that remediation, never the verifier again.
          refusal = String(entry.refusal_summary || "no summary");
          // Issue-122: the finding it was filed under, as the host recorded
          // it, and that fix's place in the ordinal -- so a fix that landed
          // replays and only what the stop cut off (its verifier) runs.
          try {
            const parsed = typeof entry.finding_json === "string" ? JSON.parse(entry.finding_json) : null;
            if (Array.isArray(parsed) && parsed.length === 1 && parsed[0] && typeof parsed[0] === "object") recorded = parsed[0];
          } catch (_) { recorded = null; }
          align.refile(Number.isInteger(entry.fix_ordinal) ? entry.fix_ordinal - 1 : null);
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
        const finding = recorded || {
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
      if (ran) align.raise();
    }
    return outcomes;
  };
  // Issue-117: the residual gaps verifiers recorded (the first pass: accepted
  // verifiers'; the third: HIGH gaps whatever the verdict, Issue-121). Before
  // acceptance the HOST names, on a checkpoint's view, each bounded round it
  // plans: the gaps, the tasks it routes them to, and the exact files no task
  // declares that the round may write. Each is ONE remediation round of its
  // own unit under the host's key (the host answers no other), recorded done
  // when it returns and never planned again. Nothing here decides a gap: the
  // final gate reads the round's records.
  const resolveResiduals = async (opts = {}) => {
    const strings = (list) => (Array.isArray(list) ? list.filter((x) => typeof x === "string" && x) : []);
    const rounds = [];
    // Issue-118: a SECOND pass plans rounds for what the first pass's own
    // rounds found (new high gaps, and red tests the host proved owed but no
    // round could write). Issue-121: a THIRD and final pass plans rounds only
    // for the HIGH gaps the second pass's own verifiers recorded. The first
    // three slots ask exactly what they always asked, so a resumed run
    // replays them. Batch O2: after the third, pass N (4, 5, ...) is asked
    // while the pass before it planned any round; the host plans pass N only
    // with work that makes progress (`residual_later_pass`), so a pass it
    // plans nothing for ends them, and what stands then blocks.
    const passOptions = [
      null,
      { residualGaps: true, task: "Residual gaps accepted verifiers recorded: the rounds the host plans before acceptance" },
      { residualGaps: true, residualPass: 2, task: "Residual gaps the host's own rounds left: its second and final pass before acceptance" },
      { residualGaps: true, residualPass: 3, task: "High residual gaps the host's second-pass rounds' verifiers recorded: its third and final pass before acceptance" },
    ];
    const align = ordinalAligner();
    const ranHere = new Set();
    const passSlot = (pass) => passOptions[pass]
      || { residualGaps: true, residualPass: pass, task: `Residual gaps pass ${pass}: what the host's previous pass left, planned only while it makes progress` };
    for (let pass = 1; ; pass += 1) {
      const view = await w.checkpoint(`residual-gaps-${pass}`, passSlot(pass));
      const plan = (view && (view.residual_plan || (view.data && view.data.residual_plan))) || [];
      const passFields = pass >= 2 ? { pass } : {};
      for (const entry of Array.isArray(plan) ? plan : []) {
        // Issue-122: a round an earlier session finished makes no call here;
        // its calls' place in the ordinal is taken as the host recorded it.
        if (entry && entry.source === "host" && entry.attempted === true && typeof entry.key === "string" && !ranHere.has(entry.key)) {
          align.to(entry.resume_ordinal);
        }
        const tasks = strings(entry && entry.task_ids);
        if (!entry || entry.source !== "host" || entry.attempted === true || typeof entry.key !== "string" || tasks.length === 0) continue;
        const files = strings(entry.expansion_files);
        // The host built the claim and checked that the prompt made from it
        // passes its own dispatch check; a round it could not is not offered.
        if (typeof entry.claim !== "string" || entry.dispatchable === false) {
          rounds.push({ key: entry.key, kind: entry.kind, taskIds: tasks, files, refused: "not dispatchable", pass });
          continue;
        }
        if (entry.kind === "adjudication") {
          // Issue-117: a HIGH gap no file round could carry: ONE read-only
          // verification of the recording unit's tasks on the tree as it is.
          const ids = [...tasks].sort();
          const contract = Object.assign({ version: 1, stage: "verify", taskId: ids.length > 1 ? crossTaskKey(ids) : ids[0], round: 1, maxRounds: 1,
            sourceReduceCallIds: ["adversarial-review-reduce", "coverage-audit-reduce"], contest: entry.key, residual: Object.assign({ key: entry.key, files: [] }, passFields) },
          ids.length > 1 ? { taskIds: ids } : {});
          const check = await dispatchAgent(`${entry.key}-adjudicate`, entry.claim, { verify: true, taskIds: ids, remediationContract: contract });
          const refused = Boolean(check && (check.residual_refused || (check.data && check.data.residual_refused)));
          // Refused at dispatch, it judged nothing: never recorded done.
          if (!refused) await w.checkpoint(`${entry.key}-done`, { task: `Residual adjudication ${entry.key} returned` });
          rounds.push({ key: entry.key, kind: entry.kind, taskIds: ids, files, accepted: accepted(check), refused, pass });
          align.raise();
          continue;
        }
        const finding = Object.assign({ id: entry.key, canonical_task_ids: tasks, severity: entry.severity || "high", claim: entry.claim }, tasks.length > 1 ? { attributable_to_task: false } : {});
        // Issue-122: a round a stop cut off files its fix where it was filed.
        align.refile(Number.isInteger(entry.fix_ordinal) ? entry.fix_ordinal - 1 : null);
        ranHere.add(entry.key);
        const remediation = await remediateFindings([finding], {
          maxRounds: 1,
          taskFileFor: opts.taskFileFor,
          targetFilesFor: opts.targetFilesFor,
          contestKey: entry.key,
          residual: Object.assign({ key: entry.key, files, verifyNote: typeof entry.disposition_instruction === "string" ? entry.disposition_instruction : "" }, passFields),
        });
        // Refused at dispatch, nothing it planned landed: never recorded done,
        // so a later session plans it again and the gate reports it.
        if (!remediation.residualRefused) await w.checkpoint(`${entry.key}-done`, { task: `Residual round ${entry.key} returned` });
        rounds.push({ key: entry.key, kind: entry.kind, taskIds: tasks, files, remediation, pass });
        align.raise();
      }
      if (pass >= 3 && !(Array.isArray(plan) && plan.length > 0)) break;
    }
    // A round whose fix landed nothing and that no verifier judged gets ONE
    // read-only confirmation under the host's own id and contract
    // (`residual_confirm`); it mints no ordinal and writes nothing.
    const confirmView = await w.checkpoint("residual-confirm", {
      residualConfirm: true,
      task: "Residual rounds no verifier judged: one read-only confirmation each before acceptance",
    });
    const confirms = (confirmView && (confirmView.residual_confirm || (confirmView.data && confirmView.data.residual_confirm))) || [];
    for (const entry of Array.isArray(confirms) ? confirms : []) {
      const ids = strings(entry && entry.task_ids);
      if (!entry || entry.source !== "host" || entry.attempted === true || typeof entry.key !== "string"
        || typeof entry.claim !== "string" || !entry.contract || typeof entry.contract !== "object" || ids.length === 0) continue;
      const check = await dispatchAgent(`${entry.key}-confirm`, entry.claim, { verify: true, taskIds: ids, remediationContract: entry.contract });
      rounds.push({ key: entry.key, kind: "confirmation", taskIds: ids, accepted: accepted(check) });
    }
    return rounds;
  };
  const acceptance = async (opts = {}) => {
    if (acceptanceRan) {
      throw new Error("acceptance() runs once, as the final stage after review remediation; it re-runs failing checks itself");
    }
    acceptanceRan = true;
    // REM-14: a failed completion no review pass took is remediated first.
    await routeCompletionBlocked();
    const contests = await resolveContests(opts);
    await resolveResiduals(opts);
    // Batch O: no round cap of the script's own. The host decides when the
    // loop ends (`final`), on a budget that follows progress; `maxRounds`
    // is only the hint it has always been sent.
    const maxRounds = Math.max(1, Number(opts.maxRounds) || 3);
    const rounds = [];
    let checkIds = [];
    let last = null;
    // Issue 288: the stall belt compares a round with the last round that
    // actually RAN checks, and a repeat pauses the run: it never ends the loop.
    let basis = null;
    let sentSinceBasis = true;
    for (let round = 1; ; round += 1) {
      last = await w.tool(`acceptance-contract-run-${round}`, {
        tool: "acceptance-contract-run",
        round,
        maxRounds,
        checkIds,
      });
      const failing = acceptanceFailing(last);
      const entry = { round, failing_check_ids: failing.map((f) => f.check_id), remediation: null };
      rounds.push(entry);
      // Issue 288: only the host ends the loop (`final: true`). A reply with
      // no flag, or one held open with nothing failing and no operational
      // error, is a host-contract fault, never a completion: the run pauses
      // and a resumed run runs the round again. Issue 320: a round open on
      // operational errors goes on; the host's ledger pauses repeats.
      if (last.final === true) break;
      if (last.final !== false || (failing.length === 0 && !(Array.isArray(last.operational_errors) && last.operational_errors.length > 0))) {
        await w.pause(`acceptance-host-contract-${round}`, { reason: "host_contract", round, final: last.final === undefined ? null : last.final, failing_check_ids: failing.map((f) => f && f.check_id),
          recovery: "The run is paused, not failed: the host's acceptance reply neither ended the loop (final: true) nor named anything to run again. Resume the run; the acceptance loop runs the round again." });
      }
      if (failing.length === 0) { checkIds = []; continue; }
      // Belt to the host's budget: a round that saw exactly what the last
      // round that ran checks saw, with nothing sent between them, made no
      // progress. A round that evaluated no check (every failing one is an
      // "error") says nothing about the product: it is never compared. A stall
      // PAUSES the run with its evidence; a resumed run goes on with the loop.
      const passedNow = Array.isArray(last.passed) ? last.passed : [];
      const ranChecks = passedNow.length > 0 || failing.some((f) => f && f.status !== "error");
      if (ranChecks) {
        const seen = JSON.stringify(failing.map((f) => [f && f.check_id, f && f.status]));
        if (basis && seen === basis.seen && !sentSinceBasis) {
          await w.pause(`acceptance-stall-${round}`, {
            reason: "no_progress",
            round,
            compared_with_round: basis.round,
            failing_check_ids: failing.map((f) => f && f.check_id),
            recovery: "The run is paused, not failed: this acceptance round failed exactly the checks the last round that ran checks failed, and nothing was sent to repair them in between. Repair what the failing checks name, then resume the run; the acceptance loop goes on.",
          });
        }
        basis = { seen, round };
        sentSinceBasis = false;
      }
      // Issue-114: a check the host showed regressed at a run landing also
      // goes to that landing's tasks -- the owners may not write the change
      // that broke it. Absent that, exactly the owners, as before.
      const brokeIt = (f) => (f && f.regressed_by && Array.isArray(f.regressed_by.tasks) ? f.regressed_by.tasks.filter((x) => typeof x === "string" && x) : []);
      // Batch E: and to the tasks that can WRITE the files its failure
      // implicates (owners, or the tasks naming a file no task declares),
      // with those unowned files granted to the unit. Absent routing, as before.
      const routed = (f, key) => (f && f.routing && Array.isArray(f.routing[key]) ? f.routing[key].filter((x) => typeof x === "string" && x) : []);
      // Batch O2: stored project data the host granted (on its ledger); it
      // lands through the project inputs, so it is never a write target here.
      const storedData = (f) => (f && f.routing && f.routing.project_grants && typeof f.routing.project_grants === "object" ? Object.keys(f.routing.project_grants).sort() : []);
      // Issue-226: where each granted stored-data file lands (its tree, its
      // path there, and by which landing), as the host states it.
      const landings = (f) => (f && f.routing && f.routing.stored_data_landings && typeof f.routing.stored_data_landings === "object" ? f.routing.stored_data_landings : {});
      const storedLandings = (f) => [...new Set([...storedData(f), ...Object.keys(landings(f))])].sort().map((p) => `${p} lands in ${typeof landings(f)[p] === "string" ? landings(f)[p] : "the project root, through the host's audited project-input landing"}`);
      const extra = (f) => [...brokeIt(f), ...routed(f, "writer_tasks").filter((t) => !(Array.isArray(f.owning_tasks) && f.owning_tasks.includes(t)))];
      const taskSet = (f) => (extra(f).length > 0 ? [...new Set([...(Array.isArray(f.owning_tasks) ? f.owning_tasks : []), ...extra(f)])] : f.owning_tasks);
      // Issue-128: only a check that RAN and FAILED is a task's to fix. One
      // that could not be evaluated (status "error": a scratch that could not
      // be built, a site failure) is the host's environment, and a contract
      // defect is the contract's.
      // Batch J: nor is one the host marked `blocked` -- no unit can fix it;
      // the host raised it as an operational finding instead.
      // Batch O: unless the host re-routed it (`remediable`: an error it
      // repaired into a task's failure, a blocked check whose files it
      // granted, an unowned check it assigned).
      const ranAndFailed = (f) => f && ((f.status === "failed" && f.contract_defect !== true && !(typeof f.blocked === "string" && f.blocked)) || f.remediable === true);
      // Batch J: a check the host could not tie to a landing goes to its
      // owners with what the regression search established.
      const searchNote = (f) => (brokeIt(f).length === 0 && f.regression_search && typeof f.regression_search.note === "string" && f.regression_search.note ? `REGRESSION SEARCH: ${f.regression_search.note}.\n` : "");
      const sharedProbe = (f) => (typeof f.regressed_by.probed_as === "string" && f.regressed_by.probed_as ? ` (found by probing ${f.regressed_by.probed_as}, which fails identically)` : "");
      const owned = failing.filter((f) => ranAndFailed(f) && ((Array.isArray(f.owning_tasks) && f.owning_tasks.length > 0) || extra(f).length > 0));
      // Nothing a unit can fix this round, yet the host says the loop goes
      // on (it repairs the environment or re-authors a check itself): the
      // next round re-runs what failed.
      if (owned.length === 0) { checkIds = failing.map((f) => f.check_id); continue; }
      // Batch O: a key two check ids cannot share (`slug` cuts at 40).
      const slugs = owned.map((f) => slug(f.check_id));
      const ambiguous = new Set(slugs.filter((s, i) => s.length >= 40 || slugs.indexOf(s) !== i));
      const findingKey = (f) => (ambiguous.has(slug(f.check_id)) ? `acceptance-${slug(f.check_id)}-${keyHash(String(f.check_id))}` : `acceptance-${slug(f.check_id)}`);
      const findings = owned.map((f) => ({
        id: findingKey(f),
        canonical_task_ids: taskSet(f),
        // Owners, the landing that broke it and the files' writers fix it
        // together: one unit over all their files, one verifier over all.
        ...(extra(f).length > 0 && taskSet(f).length > 1 ? { attributable_to_task: false } : {}),
        severity: "high",
        source: "acceptance-contract",
        // Batch K2: the goal is the criterion, never a green check -- a unit
        // told to "make this check pass" registered a test fixture as data.
        description: `Frozen acceptance check ${f.check_id} FAILED against the finished repository: ${String(f.criterion || "")}\nkind: ${f.kind || "command"}; exit: ${f.exit_code === undefined || f.exit_code === null ? "none" : f.exit_code}${f.operational_error ? `; error: ${String(f.operational_error)}` : ""}\n${f.frozen_check ? "The FROZEN CHECK the harness runs is given verbatim after these findings.\n" : ""}stderr (its end and every failure line): ${String(f.stderr_tail || "")}\nstdout (its end and every failure line): ${String(f.stdout_tail || "")}\n${brokeIt(f).length > 0 ? `REGRESSION: it held at ${f.regressed_by.held_at} and first failed at run landing ${f.regressed_by.landing_commit} (${f.regressed_by.landing_stage}), landed by ${brokeIt(f).join(", ")}${sharedProbe(f)}; find what that change broke and repair the product there so it meets the criterion again.\n` : ""}${searchNote(f)}${routed(f, "implicated_files").length > 0 ? `IMPLICATED FILES: ${routed(f, "implicated_files").join(", ")}${routed(f, "granted_files").length > 0 ? `; granted to this unit (no task declares them): ${routed(f, "granted_files").join(", ")}` : ""}.\n` : ""}${storedLandings(f).length > 0 ? `STORED DATA granted to this unit: ${storedLandings(f).join("; ")}.\n` : ""}GOAL: the product must genuinely meet the criterion this check tests: ${String(f.criterion || "")}\nA green check is not the goal; it is only the evidence. Passing it by any other means counts as a failure, and the verifier that judges this fix will refuse it. That includes: editing or weakening the check; adding or registering test, fixture, sample, placeholder or hand-made data as the product's data (its datasets, registries or runtime inputs); special-casing the check's inputs; weakening validation; or restoring or hand-writing generated outputs or data files to match what the check expects. If the product cannot genuinely meet the criterion within this unit's scope, do not force a pass: return an honest blocked status with the evidence of what stands in the way.`,
      }));
      // Batch O: on the host's first stalled round (`escalate`), every
      // failing check goes to ONE unit over all their owners together.
      if (last.escalate === true && findings.length > 0) {
        const everyone = [...new Set(findings.flatMap((f) => (Array.isArray(f.canonical_task_ids) ? f.canonical_task_ids : [])))].sort();
        for (const f of findings) {
          f.canonical_task_ids = everyone;
          if (everyone.length > 1) f.attributable_to_task = false;
        }
      }
      const hostGrants = {};
      for (const f of owned) if (routed(f, "granted_files").length > 0) hostGrants[findingKey(f)] = routed(f, "granted_files");
      const frozenChecks = {};
      for (const f of owned) if (typeof f.frozen_check === "string" && f.frozen_check) frozenChecks[findingKey(f)] = f.frozen_check;
      sentSinceBasis = true;
      entry.remediation = await remediateFindings(findings, {
        hostGrants,
        hostEvidence: true,
        frozenChecks,
        maxRounds: 1,
        // REM-13: a script that names no task files or targets (one the
        // prelude runs acceptance for) gets the host's: each task's own file
        // and declared files, from this round's reply.
        taskFileFor: typeof opts.taskFileFor === "function" ? opts.taskFileFor : (id) => (last && last.task_files && typeof last.task_files[id] === "string" ? last.task_files[id] : ""),
        targetFilesFor: typeof opts.targetFilesFor === "function" ? opts.targetFilesFor : (id) => (last && last.task_scope && Array.isArray(last.task_scope[id]) ? last.task_scope[id] : undefined),
        sourceReduceCallIds: opts.sourceReduceCallIds,
        // Batch H: these findings are this round's observation, which the
        // host re-runs on every resume; an answer to an earlier run of it
        // (the same failure seen before a fix that did not hold) is not an
        // answer to this one.
        observedBy: [`acceptance-contract-run-${round}`],
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
      // Batch J: failed checks no unit can fix, each with the host's rule.
      blocked: failing.filter((f) => typeof f.blocked === "string" && f.blocked),
      passed: (last && Array.isArray(last.passed)) ? last.passed.slice() : [],
    };
  };

  // REM-13: whether the script ran the acceptance stage (the prelude runs it
  // for one that returned without it).
  const acceptanceCalled = () => acceptanceRan;

  return Object.freeze({ agent, agents, phase, log, pipeline, adversarialReview, coverageAudit, remediateFindings, remediationBudget, resolveContests, resolveResiduals, acceptance, acceptanceCalled, accepted, usable, outcomesOf, reviewFindings, w });
}
