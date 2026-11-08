function supplementaryRequirement(id) {
  if (typeof id !== "string" || !id.startsWith("SUP-")) return null;
  const requirement = id.slice(4).replace(/-R[0-9]+$/, "");
  return requirement.length > 0 && !/[^A-Za-z0-9-]/.test(requirement) ? requirement : null;
}
const COMMAND_EXAMPLES = 'Command examples: {"kind":"command","cwd":"repo_root","command":"make test"} and {"kind":"command","cwd":"project_root","command":"./verify"}.';

const ACCEPTANCE_SHAPE = JSON.stringify({
  schema_version: 1,
  prd: { path: "", digest: "" },
  gap_policy: { permitted_acceptance_ids: [], forbidden_phrases: [], required_fields: [] },
  acceptance: [
    {
      id: "<exact acceptance id defined by the PRD>",
      criterion: "",
      check: {
        kind: "floor",
        contract: {
          kind: "<deliverable kind>",
          artifact_path: "<repository-relative artifact path>",
          artifact_format: "json",
          required_true_fields: ["<field that must be true>"],
          typed_verifier_command: "<command that exercises the deliverable and fails when the criterion is false>"
        }
      },
      gap_permitted: false,
      covers: ["<coverable id from the PRD whose violation makes this check fail>"],
      judgment: { verdict: "accepted", counterexample: "", reason: "", host_call_id: "" }
    },
    {
      id: "<exact acceptance id defined by the PRD>",
      criterion: "",
      check: {
        kind: "command",
        // Live replies put long scripts in check blocks; inline scripts break JSON parsing.
        command_block: true,
        cwd: "<repo_root or project_root>"
      },
      gap_permitted: false,
      covers: ["<coverable id from the PRD whose violation makes this check fail>"],
      judgment: { verdict: "accepted", counterexample: "", reason: "", host_call_id: "" }
    }
  ],
  supplementary: []
});

// The per-entry author gets the entries alone. Showing it the whole contract as
// the example is what made it return the whole contract (observed live).
const ENTRY_SHAPES = JSON.stringify(JSON.parse(ACCEPTANCE_SHAPE).acceptance);

function deliverableContractRule() {
  if (typeof args.deliverableContractRule !== "string" || !args.deliverableContractRule.trim()) {
    throw new Error("host did not supply deliverableContractRule");
  }
  return args.deliverableContractRule;
}

const SKELETON_SHAPE = JSON.stringify({
  schema_version: 1,
  acceptance_digest: "",
  tasks: [{
      task_id: "<canonical task id>",
      file_name: "<canonical task id>.md",
      depends_on: [{
          task_id: "<canonical id of the task depended on>",
          consumes: [{
            artifact_path: "<path this task reads from that one>",
            instance_source_records_field: "<optional: exact instance_source_records_field value from the named producer contract>",
            registry_records_field: "<optional: exact registry_records_field value from the named producer contract>"
          }],
          ordering_only: false
        }
      ],
      blocks: ["<canonical id of a task that waits on this one>"],
      implements: ["<obligation or constraint id the host lists>"],
      source_sections: ["<PRD section key the host lists>"],
      deliverable_contracts: [{
          kind: "<short stable name for what this task produces>",
          artifact_path: "<path this task produces, exactly as its dependents consume it>",
          min_instances: 1
        }]
    }]
});

// Frozen fields are host-stamped at landing; the body author supplies the
// sections below them and does not need to reproduce the frozen values.
const BODY_SHAPE = [
  "```yaml",
  "task_id: <frozen task id>",
  "title: <short title>",
  "complexity: low|medium|high",
  "status: ready",
  "depends_on: <host stamps frozen value>",
  "blocks: <host stamps frozen value>",
  "implements: <host stamps frozen value>",
  "source_sections: <host stamps frozen value>",
  "required_env_keys: []",
  "required_tools: <exact tools this task must exercise; [] only when no tool invocation is required>",
  "deliverable_contracts: <host stamps frozen value>",
  "```"
].join("\n");

// A candidate the host could not even read is packaging, not authorship: the
// artifact it carried was never judged. The progress rule ranks it below every
// judged or refused candidate (workflow_decompose_v1_progress.js).
const PACKAGING_REFUSAL = "candidate artifact was refused: the reply is not a JSON document";

async function workflow(w) {
  requireFixedArgs();
  if (args.phaseSeed) applySeedOrdinals(); // Issue 360: after an upgrade, start from the phase seed.
  const frozen = frozenChain();

  const { acceptance, skeleton, changedChecks } = await prepareAuthorStages(w, frozen);

  // Keyed by frozen file name so a body the set gate sends back replaces its
  // earlier evidence entry instead of appending a second one. A body already
  // on disk under the frozen chain is not authored: the set gates judge it
  // with the rest, and send it back here if they find it wanting.
  const bodies = new Map();
  const unauthored = [];
  const forceBodySubjects = new Set(Array.isArray(args.forceBodySubjects) ? args.forceBodySubjects : []);
  for (const subject of skeleton.subjects) {
    requireSubject(subject);
    if (frozenBodyReusable(frozen, skeleton, subject, changedChecks) && !forceBodySubjects.has(subject.taskId)) continue;
    unauthored.push([subject, []]);
  }
  const chain = { outcome: skeleton, subjects: skeleton.subjects };
  await authorBodies(w, unauthored, bodies, chain);

  // The skeleton the set gates stand on: re-frozen by the loop when a
  // finding only the skeleton can repair is open (Batch O).
  const gates = await runSetGateLoop(w, chain, bodies);
  const evidence = [acceptance, chain.outcome, ...bodyEvidence(chain.subjects, bodies), gates.taskSetLint, gates.requirementsTrace];
  reconcile(evidence, bodies.size);

  return await w.finalReport("fixed-decomposition-final", {
    status: "accepted",
    inputs: evidence,
    task: "Reconcile the host-validated fixed decomposition evidence."
  });
}

function requireFixedArgs() {
  if (!args || typeof args !== "object") throw new Error("fixed decomposition args are absent");
  for (const key of ["projectRoot", "repositoryRoot", "prdPath", "prdDigest", "taskRoot"]) {
    if (typeof args[key] !== "string" || args[key].trim() === "") {
      throw new Error(`fixed decomposition argument ${key} is missing`);
    }
  }
  if (args.gateMode !== "observe" && args.gateMode !== "enforce") {
    throw new Error("fixed decomposition requires observe or enforce gate mode");
  }
}

// Author calls already made per phase. A body the set gate sends back keeps
// its call-id family and continues the ordinal, so on resume its earlier
// attempts replay as history and a repair never overwrites a recorded call.
const AUTHOR_CALLS = new Map();

// One subject's author loop, limited by progress (Issue 261): an attempt that
// makes progress keeps it going; STALL_ATTEMPTS in a row without progress
// advances the authored repair rung. Rung progress follows replayed calls.
async function authorCandidate(w, policy) {
  let feedback = Array.isArray(policy.initialFeedback) ? policy.initialFeedback.slice() : [];
  // Seeded feedback is attempt 0 of the history: every later prompt in this
  // phase keeps showing the finding the phase was opened to repair.
  const history = feedback.length > 0 ? [{ attempt: 0, findings: feedback.slice() }] : [];
  let call = AUTHOR_CALLS.get(policy.phase) || 0;
  let attempt = 0;
  let bodyCandidate = null; // Issue 601: retained only within this author/land episode.
  let lastFindings = feedback.slice();
  // Issue 551: the findings open in the latest gate result. Earlier findings
  // stay in `history`, shown as ones not to regress, never as current.
  let openFindings = feedback.slice();
  let judgeFeedback = [];
  // #552: the latest round's freeze judged nothing (an unusable judge reply).
  let judgedNothing = false;
  let repairRung = Number.isInteger(policy.initialRung) ? policy.initialRung : 1;
  let repairCycle = 1;
  const progress = newProgress(feedback);
  const authorState = { entries: new Map(), retryIds: null, retryPreparation: true };
  // #555: findings the loop opens with that name no entry are phase-level;
  // a frozen stage's repair attributes its own to each entry (repairFrozenStage).
  if (policy.author) authorState.phaseFindings = Array.isArray(policy.initialFeedback) ? policy.initialFeedback.slice() : [];
  // Issue 360: after an upgrade, start from the latest durable candidate (workflow_decompose_v1_seed.js).
  const seeded = args.phaseSeed && !seedWouldReplaceLiveFeedback(policy)
    ? seedAuthorLoop(policy, authorState)
    : null;
  if (seeded) [feedback, lastFindings, openFindings] = [seeded.feedback.slice(), seeded.feedback.slice(), seeded.feedback.slice()];
  let carried = seeded ? seeded.carried : null;
  let carriedLandings = seeded?.retainedLandings === true;
  let previousCandidate = skeletonPreviousCandidate(policy, seeded ? seeded.previousCandidate : policy.previousCandidate, seeded);
  let lastAcceptedCandidate = previousCandidate;
  for (;;) {
    const stall = stallReason(progress);
    if (stall) {
      if (stall === "no_progress" && policy.deferAuthoredPending) {
        // Yield the stalled subject so independent bodies settle before the
        // pool pauses. A resume advances its ladder with a fresh window.
        return { pendingAuthor: true, pendingRung: repairRung, findings: lastFindings.slice() };
      }
      // A host fault is not an authored finding and cannot be repaired here.
      bodyCandidate = null;
      await pauseAuthorLoop(w, policy.phase, progress, stall, lastFindings);
      continue;
    }
    if (progress.history.length > 0 && progress.history.at(-1).kind !== "operational") {
      // Deferred bodies spend the existing window on one complete strategy cycle.
      // The first observation is the baseline; the next attempts visit rungs 2–4.
      // A rung change consumes an attempt; it never opens a fresh window.
      repairRung = repairRung === 4 ? 2 : repairRung + 1;
      if (repairRung === 2) {
        repairCycle += 1;
        // Entry authors keep their own refusal in authorState; do not copy
        // a sibling's finding into another entry's fresh-context prompt.
        // Other authors keep their open findings and the history that marks
        // repeats and earlier findings (Issue 551).
        // A round that judged nothing keeps the history (#552).
        if (policy.author) {
          feedback = judgeFeedback.slice();
          if (!judgedNothing) history.length = 0;
        }
      }
      await repairEvent(w, policy.phase, repairRung, repairCycle, call, progress.calls);
    }
    const carry = carried;
    carried = null;
    const nextCall = carry === null ? call + 1 : call;
    const addedBefore = authorState.added || 0;
    const rungInstruction = repairRung === 1 ? ""
      : repairRung === 2 ? "\nRe-author a complete replacement from fresh context. Keep the subject contract and repair every current finding."
      : repairRung === 3 && policy.author ? "\nReturn a complete replacement of this entry that repairs every finding listed below for this entry and keeps every earlier one listed for it satisfied."
      : repairRung === 3 ? "\nReturn a complete replacement. Work through the findings in smaller groups while preserving every finding from this and earlier attempts."
      : policy.author ? "\nReturn a complete replacement of this entry that repairs only the findings listed below for this entry."
      : "\nThis subject is deferred while independent subjects settle. Return a complete replacement when re-entered with the latest findings.";
    const grouping = bodyCandidate ? bodyCandidate.findings.map(findingText) : feedback;
    const midpoint = Math.ceil(grouping.length / 2);
    const groupFeedback = repairRung === 3 && grouping.length > 1
      ? (nextCall % 2 === 1 ? grouping.slice(0, midpoint) : grouping.slice(midpoint))
      : grouping;
    // #555: the part of an entry prompt every entry shares names no finding:
    // each entry reads only its own, and the phase-level ones, in its own
    // section (authorOne). The round's findings, its history and its groups
    // belong to every subject at once, so they stay out of it.
    const promptFeedback = policy.author ? [] : [...new Set([...feedback, ...openFindings, ...(policy.initialFeedback || [])])];
    const group = repairRung === 3 && !policy.author ? `\nPrioritize this group first:\n- ${groupFeedback.join("\n- ")}` : "";
    let authored;
    try {
      if (carry !== null) authored = { status: "accepted", stopReason: "end_turn", content: carry, retainedLandings: carriedLandings };
      else {
        const operation = `\nRepair operation r${repairRung}, cycle ${repairCycle}, authored call ${nextCall}.`;
        const bodyRepair = bodyRepairContext(policy, repairRung, bodyCandidate);
        const prompt = authorPrompt(`${policy.prompt(previousCandidate)}${bodyRepair ? bodyRepair.text : rungInstruction}${operation}${group}`, attempt + 1,
          bodyRepair ? [...new Set([...bodyRepair.feedback, ...(policy.initialFeedback || [])])] : promptFeedback, bodyRepair ? bodyRepair.history : policy.author ? [] : history);
        // Context writes are not journaled. Reserve no ordinal until the
        // complete prompt is ready, so recovery and replay dispatch identically.
        const task = policy.author ? prompt : await requireDispatchable(w, `${policy.phase}-prompt`, prompt);
        if (policy.author) {
          authored = await policy.author(w, task, nextCall, authorState);
          if (authorState.roundCalls > 0 || authored.status !== "failed") {
            call = nextCall;
            AUTHOR_CALLS.set(policy.phase, call);
          }
        } else {
          progress.calls += 1;
          call = nextCall;
          AUTHOR_CALLS.set(policy.phase, call);
          authored = await w.agent(`${policy.phase}-author-${call}`, {
            task, tier: "planner", resultMode: "rawOutcome",
            ...(policy.phase === "skeleton" ? { recordLanding: "skeleton", openFindings: openFindings.slice(), previousSkeleton: skeletonLandingSeed(previousCandidate), frozenSkeleton: frozenChain().skeleton || CURRENT_SKELETON_TASKS !== null } : {})
          });
        }
      }
    } catch (error) {
      if (error?.retainedBodyContext === true && typeof w.checkpoint === "function") {
        const message = `retained body candidate context failed for ${policy.phase}: ${error.message}`;
        await w.checkpoint(`${policy.phase}-context-failed-${nextCall}`, { task: message });
      }
      if (!error || error.authorContext !== true) throw error;
      authorState.roundCalls = 0;
      authorState.roundAnswered = 0;
      authorState.roundPassed = [];
      authorState.roundOutage = true;
      authored = { status: "failed", summary: error.message };
    }
    const measuredReplies = policy.author && authorState.roundCalls !== undefined;
    if (measuredReplies) {
      progress.calls += authorState.roundCalls;
      progress.answered += authorState.roundAnswered;
    }
    // A round without a candidate can still retain work when a sibling fails:
    // a previously missing entry lowers the outstanding-entry count. A
    // rewrite alone clears no defect; every failure shares the same window.
    const advanced = (authorState.added || 0) > addedBefore;
    const round = authorRoundReport(policy, authorState);
    if (policy.phase === "skeleton") authored = refuseUnchangedSkeleton(authored, carry, openFindings);
    if (authored.invalid_answer === true) authored = { ...authored, malformed: true };
    if (policy.phase === "skeleton" && authored.retainedLandings === true && typeof authored.content === "string") {
      previousCandidate = authored.content;
    }
    if (authored.status === "failed") {
      // Every kind of failed round credits the entries that passed in it and
      // counts any outage in it. Each entry reads its own refusal or note;
      // the shared feedback stays the gate's findings.
      if (authored.refusals) {
        recordRepairs(progress, call, authored.refusals, round, !measuredReplies, advanced);
        lastFindings = authored.findings.map(progressText);
      } else if (authored.malformed) {
        recordAnswered(progress, call, policy.phase === "skeleton" ? "malformed" : "entries", advanced, !measuredReplies, round);
        feedback = [...openFindings, authored.summary || "malformed replies"];
        lastFindings = feedback.slice();
      } else {
        recordOperational(progress, call, authored.summary, advanced, round, "provider");
        lastFindings = [`author call failed operationally: ${authored.summary || "no summary"}`];
      }
      if (authored.refusals || authored.malformed) attempt += 1;
      continue;
    }
    if (authored.stopReason !== "end_turn" || typeof authored.content !== "string" || authored.content.length === 0) {
      attempt += 1;
      feedback = [authored.stopReason === "max_tokens"
        ? incompleteReplyFeedback(authored.content)
        : `Provider outcome was incomplete (stopReason=${authored.stopReason || "missing"}); return one complete artifact.`];
      lastFindings = feedback.slice();
      recordAnswered(progress, call, "incomplete", false, !measuredReplies);
      continue;
    }

    const outcome = await w.hostCommand(policy.capability, {
      stdin: authored.content,
      ...(policy.target ? { target: policy.target } : {})
    });
    // Skeleton landing calls reach this gate with host-composed records.
    // Retained landings survive pre-gate refusals; other candidates need acceptance.
    if (policy.phase === "skeleton") {
      if (acceptedSkeletonCandidate(authored.content, outcome)) lastAcceptedCandidate = authored.content;
      previousCandidate = lastAcceptedCandidate;
    }
    if (policy.author && outcome && outcome.publicationReceipt) clearPublishedDrafts(authorState);
    const routed = routeFindings(outcome, policy.retryScopes, policy.shadowScopes);
    if (policy.capability === "land-task-body" && routed.shadow.length > 0 && routed.fatal.length === 0 && !routed.operational) {
      return { ...outcome, skeletonFindings: routed.shadowFindings, bodyFindings: routed.retryFindings, shadowRung: repairRung };
    }
    // #554: a check the host could not prove, on an entry this loop
    // authored, goes to its author once the next freeze reports it again on
    // the entry's unchanged text; until then the host probes it again.
    const unproven = routed.operationalKind === "authored_judge_reply"
      ? { repeated: new Set(), subjects: new Set() }
      : unprovenAuthored(authorState, outcome, routed);
    if (routed.operational && unproven.repeated.size > 0) {
      routed.operational = null;
      for (const finding of unproven.repeated) {
        const text = String(finding.text || "unnamed policy finding");
        routed.all.push(text);
        routed.allFindings.push(finding);
        routed.fatal.push(text);
        routed.fatalFindings.push(finding);
        routed.fatalOperational[routed.fatal.length - 1] = true;
      }
    }
    if (routed.operational) {
      if (routed.operationalKind === "authored_judge_reply") {
        // The judge could not use this candidate. Feed its reason to the
        // author so the next call creates a replacement.
        // #552: the host names the checks the judge left undecided as data.
        // The text match is kept only for an envelope saved before that.
        if (policy.author) {
          const subjects = outcome.gateEnvelope.operational_error.subjects;
          const listed = Array.isArray(subjects) && subjects.length > 0 ? subjects.map(String) : null;
          const named = listed ? null : /still have no [^(]*\(([^)]*)\)/.exec(routed.operational);
          const namedIds = new Set(listed || (named ? named[1].split(",").map(id => id.trim()) : []));
          const retryIds = (authorState.order || []).filter(id => namedIds.has(id));
          authorState.retryIds = retryIds.length > 0 ? new Set(retryIds) : null;
          // #552: the open findings of the last gate that judged stay current;
          // the note goes only to the entries it names.
          // A check the host already knew it could not judge has its own
          // finding in the envelope: that is its note, never another's.
          const note = `freeze judge reply unusable: ${routed.operational}`;
          const own = new Map();
          for (const finding of outcome.gateEnvelope.policy_findings || []) {
            own.set(finding.subject, [own.get(finding.subject), String(finding.text || "")].filter(Boolean).join("\n- "));
          }
          authorState.judgeNotes = new Map([...(authorState.retryIds || authorState.order || [])].map(id => [id, own.get(id) || note]));
        } else {
          feedback = [`freeze judge reply unusable: ${routed.operational}`];
          judgeFeedback = feedback.slice();
        }
        judgedNothing = true;
        recordAnswered(progress, call, "authored_judge_reply", advanced, !measuredReplies, round);
        lastFindings = [`freeze judge reply unusable: ${routed.operational}`];
        continue;
      }
      // The gate never judged the candidate; only the round that made it can
      // be progress: a pass in it, or a previously missing entry it added.
      if (!measuredReplies) progress.answered += 1;
      recordOperational(progress, call, routed.operational, advanced, round, policy.capability);
      lastFindings = [`${policy.capability}: ${routed.operational}`];
      carried = authored.content;
      carriedLandings = authored.retainedLandings === true;
      continue;
    }
    const ownedFound = routed.fatalFindings.filter((finding) => {
      if (finding.remediation_scope === "operational") return unproven.repeated.has(finding);
      const source = typeof finding.source_path === "string" ? finding.source_path.replace(/\\/g, "/") : "";
      if (policy.target && authoredSourceMatches(source, policy.target.fileName)) return true;
      return policy.phase === "acceptance" && (source === `${String(args.taskRoot || "").replace(/\\/g, "/")}/acceptance-contract.json` || source === "acceptance-contract.json")
        && typeof finding.subject === "string"
        && (Object.prototype.hasOwnProperty.call(args.acceptanceCriteria || {}, finding.subject)
          || authorState.entries.has(finding.subject));
    });
    const ownedFatal = ownedFound.map((finding) => unproven.repeated.has(finding)
      ? { ...finding, text: `the host could not prove this check on two freezes of its unchanged text, so a host retry does not help: change the check so that the host can run it to its own verdict. The host reported: ${finding.text}` }
      : finding);
    if (ownedFatal.length > 0) {
      const owned = new Set(ownedFound);
      const ownedText = ownedFatal.map((finding) => String(finding.text || "unnamed policy finding"));
      routed.fatal = routed.fatal.filter((_, index) => !owned.has(routed.fatalFindings[index]));
      routed.fatalOperational = routed.fatalOperational.filter((_, index) => !owned.has(routed.fatalFindings[index]));
      routed.fatalFindings = routed.fatalFindings.filter((finding) => !owned.has(finding));
      routed.retry.push(...ownedText);
      routed.retryFindings.push(...ownedFatal);
    }
    if (policy.capability === "land-task-body") {
      bodyCandidate = retainBodyCandidate(bodyCandidate, policy, authored.content, routed.retryFindings,
        [...history, { attempt: attempt + 1, findings: routed.retry.slice() }]);
    }
    if (policy.author) {
      const ids = new Set(Object.keys(args.acceptanceCriteria || {}));
      const repair = routed.retryFindings;
      const latest = attributeEntryFindings(repair, ids, Boolean(outcome.publicationReceipt), JSON.parse(authored.content), authorState.entries, policy.capability);
      settleGateFindings(authorState, latest.own);
      settlePhaseFindings(authorState, latest.phase);
      authorState.judgeNotes = null;
      const stale = repair.find(finding => finding?.deterministic_defect?.code === "stale_acceptance_contract");
      if (stale) {
        // The contract changed after this freeze was prepared: rebuild the
        // candidate from the current published contract the host returned.
        // With nothing else to repair the rebuilt candidate is frozen again at
        // once, inside the ordinary no-progress window.
        const rebuilt = refreshFromCurrentContract(authorState, outcome.gateEnvelope.report,
          repair.filter(finding => finding !== stale), ids, JSON.parse(authored.content), policy.capability);
        if (rebuilt !== null) {
          recordAnswered(progress, call, "stale_acceptance_contract", advanced, !measuredReplies, round);
          lastFindings = [String(stale.text)];
          carried = rebuilt;
          continue;
        }
      } else {
        const selected = repair.map(finding => ownedFatal.includes(finding)
          ? {...finding, text: `check '${finding.subject}': ${finding.text}`} : finding);
        authorState.retryIds = acceptanceRepairIds(selected, ids, Boolean(outcome.publicationReceipt), JSON.parse(authored.content), authorState.entries, policy.capability);
      }
    }
    // A committed artifact is not the finished one: repairable findings are
    // still fed back below in either mode, and a loop that stalls on them
    // re-authors in either mode. A check the host could not prove
    // (an operational finding) beside findings the author can repair stops
    // nothing: it blocks publication, the author repairs its own, and the
    // next freeze probes the unproven check again.
    // #554: nor does a check the host could not prove on an entry this
    // loop authored end the phase: the same candidate is frozen again, and
    // the check goes to its author if that freeze reports it again.
    const pending = routed.fatalFindings.filter((finding) => finding.remediation_scope === "operational"
      && unproven.subjects.has(finding.subject));
    const fatal = routed.fatal.filter((text, index) => !(routed.retry.length > 0 && routed.fatalOperational[index])
      && !pending.includes(routed.fatalFindings[index]));
    if (fatal.length > 0) {
      await stopFixed(`${policy.phase} stopped: ${fatal.join(" | ")}`);
    }
    if (routed.retry.length === 0 && pending.length > 0) {
      if (!measuredReplies) progress.answered += 1;
      const texts = pending.map((finding) => String(finding.text || "unnamed policy finding"));
      recordOperational(progress, call, texts.join(" | "), advanced, round, policy.capability);
      lastFindings = texts;
      carried = authored.content;
      carriedLandings = authored.retainedLandings === true;
      continue;
    }
    if (routed.retry.length === 0) {
      requireCommitted(outcome, policy.phase);
      if (policy.phase === "skeleton") recordSkeletonTasks(outcome, authored.content);
      return outcome;
    }
    attempt += 1;
    history.push({ attempt, findings: routed.retry.slice() });
    feedback = routed.retry;
    judgedNothing = false;
    if (policy.author) judgeFeedback = feedback.slice();
    lastFindings = routed.retry.slice();
    const artifactRefused = routed.retryFindings.some(f => f.remediation_scope === "candidate_artifact")
      && !routed.retryFindings.some(f => f.remediation_scope !== "candidate_artifact");
    if (!artifactRefused) openFindings = routed.retry.slice();
    else lastFindings = [...new Set([...lastFindings, ...openFindings])];
    // `advanced` is not credited here on purpose: the gate measured the whole
    // candidate, and that measure decides (Issue 261).
    recordAttempt(progress, call, routed.retryFindings, !measuredReplies, round);
    openRepairEpisode(progress);
  }
}
