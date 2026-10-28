// The launcher's reading of the task root: which frozen stages exist and
// verify, and which frozen bodies are already on disk. Absent means nothing
// is frozen. A skeleton without an acceptance contract, or bodies without a
// skeleton, is a chain the host would never have reported. `tasks` is the
// frozen skeleton's entries as published, one per subject, or null when the
// launch argument does not carry them.
function frozenChain() {
  const raw = args.frozenChain;
  const none = { acceptance: false, skeleton: false, subjects: [], bodies: new Set(), tasks: null };
  if (raw === undefined || raw === null) return none;
  if (typeof raw !== "object" || Array.isArray(raw)) throw new Error("fixed decomposition frozenChain argument is malformed");
  const acceptance = raw.acceptance === true;
  const skeleton = raw.skeleton === true;
  const subjects = Array.isArray(raw.subjects) ? raw.subjects : [];
  const bodies = new Set(Array.isArray(raw.bodies) ? raw.bodies : []);
  if (skeleton && !acceptance) throw new Error("frozenChain reports a skeleton without an acceptance contract");
  if (!skeleton && (subjects.length > 0 || bodies.size > 0)) throw new Error("frozenChain reports subjects or bodies without a frozen skeleton");
  for (const subject of subjects) requireSubject(subject);
  for (const fileName of bodies) {
    if (!subjects.some((subject) => subject.fileName === fileName)) throw new Error(`frozenChain body ${fileName} names no frozen subject`);
  }
  const tasks = raw.tasks === undefined ? null : frozenTaskEntries(raw.tasks);
  if (raw.tasks !== undefined && (!skeleton || !tasks || tasks.size !== subjects.length
    || subjects.some((subject) => !tasks.has(subject.taskId)))) {
    throw new Error("frozenChain tasks are not one frozen entry per frozen subject");
  }
  return { acceptance, skeleton, subjects, bodies, tasks };
}

// A committed outcome for a stage the launcher found frozen: the host
// re-verifies the artifact in place and reports its subjects as the freeze
// would have. Previously any finding here was fatal because there was no
// candidate to repair. An attributable authored finding now reopens its owner.
// A verification that could not run retains its host retry classification;
// typed and legacy untyped errors share the existing operational progress window.
async function verifyFrozenStage(w, capability) {
  const progress = newProgress([]);
  for (let attempt = 1; ; attempt += 1) {
    const outcome = await w.hostCommand(capability, { stdin: null });
    const routed = routeFindings(outcome, new Set(), new Set());
    if (routed.operationalKind === "permanent") await stopFixed(`${capability} stopped: ${routed.operational}`);
    if (!routed.operational) {
      if (routed.allFindings.length > 0 && routed.allFindings.every((finding) => frozenFindingOwner(finding))) {
        return await repairFrozenStage(w, outcome, routed.allFindings);
      }
      if (routed.all.length > 0) {
        await stopFixed(`${capability} stopped: ${routed.fatal.join(" | ")}`);
      }
      requireCommitted(outcome, capability);
      if (capability === "verify-frozen-skeleton") {
        const prior = frozenChain().tasks;
        let tasks = prior;
        // After an interrupted repair, verification can already pass. Compare
        // its host-read entries with the original replay inputs, not with a
        // newer publication substituted into the generation's frozen snapshot.
        try {
          const current = frozenTaskEntries(JSON.parse(outcome.gateEnvelope.report)?.tasks);
          if (current) {
            tasks = current;
            if (prior && (prior.size !== current.size || [...current].some(([id, text]) => prior.get(id) !== text))) {
              FROZEN_REPAIRS.set(outcome, { owner: "skeleton" });
            }
          }
        } catch (_) { /* Older verification reports retain their launch-bound entries. */ }
        if (tasks) SKELETON_TASKS.set(outcome, tasks);
      }
      return outcome;
    }
    progress.calls = attempt;
    recordOperational(progress, attempt, routed.operational, false, NO_ROUND, capability);
    const stall = stallReason(progress);
    if (stall) await pauseAuthorLoop(w, capability, progress, stall, [`${capability}: ${routed.operational}`]);
  }
}

// The subjects the host read at verification are the ones the launcher read,
// or the skeleton was re-frozen underneath the run. A rehearsal answers with
// a stand-in subject and is not held to this.
function requireFrozenSubjects(frozen, skeleton) {
  if (!frozen.skeleton || skeleton.dryRun === true || FROZEN_REPAIRS.has(skeleton)) return;
  const key = (subject) => `${subject.taskId}\u0000${subject.fileName}`;
  const launched = frozen.subjects.map(key).sort();
  const verified = skeleton.subjects.map(key).sort();
  if (launched.length !== verified.length || launched.some((entry, index) => entry !== verified[index])) {
    throw new Error("frozen skeleton subjects differ from the launch-bound frozenChain; the task root changed underneath the run");
  }
}

// Repair metadata is reconstructed from replayed host outcomes and author
// calls. It is never added to authoritative evidence or persisted arguments.
const FROZEN_REPAIRS = new WeakMap();

function frozenFindingOwner(finding) {
  if (typeof finding?.subject !== "string" || !finding.subject) return null;
  if (authoredSourceMatches(finding.source_path, "acceptance-contract.json")) return "acceptance";
  if (authoredSourceMatches(finding.source_path, "task-skeleton.json")) return "skeleton";
  return null;
}

function acceptanceStagePolicy(findings = []) {
  return {
    phase: "acceptance", author: authorAcceptanceEntries, capability: "freeze-acceptance",
    retryScopes: new Set(["candidate_artifact"]), prompt: acceptanceAuthorPrompt,
    initialFeedback: findings.map(findingText)
  };
}

async function repairFrozenStage(w, verified, findings) {
  const owner = frozenFindingOwner(findings[0]);
  if (findings.some((finding) => frozenFindingOwner(finding) !== owner)) {
    // Verification of one stage reports one owning artifact. Keep all mixed
    // ownership findings pending by repairing the predecessor first.
    return await repairFrozenStage(w, verified, findings.filter((finding) => frozenFindingOwner(finding) === "acceptance"));
  }
  if (owner === "skeleton") {
    const repaired = await authorCandidate(w, skeletonPolicy(findings.map(findingText)));
    FROZEN_REPAIRS.set(repaired, { owner });
    return repaired;
  }
  const policy = acceptanceStagePolicy(findings);
  let contract = null;
  try { contract = JSON.parse(verified.gateEnvelope.report); } catch (_) { /* No retained entries: re-author the complete contract. */ }
  const listed = [...(contract?.acceptance || []), ...(contract?.supplementary || [])];
  let initialized = false;
  let finalState = null;
  policy.author = async (host, prompt, round, state) => {
    if (!initialized) {
      initialized = true;
      state.entries = new Map(listed.map((entry) => [entry.id, entry]));
      for (const entry of contract?.supplementary || []) {
        const requirement = /^SUP-(REQ-[A-Za-z0-9-]+)$/.exec(entry.id)?.[1];
        if (requirement) owedSupplementary().set(entry.id, { requirement, text: currentRequirementText(requirement) });
      }
      state.retryIds = acceptanceRepairIds(findings, new Set(listed.map((entry) => entry.id)), true, contract, state.entries);
    }
    finalState = state;
    return await authorAcceptanceEntries(host, prompt, round, state);
  };
  const repaired = await authorCandidate(w, policy);
  // Every reported check remains an affected dependency, even when a judge
  // accepts its unchanged check after re-authoring. Unknown attribution reaches
  // all bodies rather than silently retaining an unproven dependency.
  const ids = acceptanceRepairIds(findings, new Set(listed.map((entry) => entry.id)), true, contract);
  const affected = ids ? new Set(ids) : null;
  if (affected && finalState) {
    for (const [id, entry] of finalState.entries) {
      const prior = listed.find((prior) => prior.id === id);
      if (!prior || acceptanceEntryKey(prior) !== acceptanceEntryKey(entry)) affected.add(id);
    }
  }
  const changedChecks = affected ? new Set([...affected].flatMap((id) => {
    const prior = listed.find((entry) => entry.id === id);
    const next = finalState?.entries.get(id);
    return [id, ...(prior?.covers || []), ...(next?.covers || [])];
  })) : null;
  FROZEN_REPAIRS.set(repaired, { owner, changedChecks });
  return repaired;
}

async function prepareAuthorStages(w, frozen) {
  // A stage the launcher found frozen and verified is not re-authored: the
  // host verifies it in place and that committed outcome is its evidence.
  // A verification finding supplies the owning author's initial feedback.
  let acceptance = frozen.acceptance ? await verifyFrozenStage(w, "verify-frozen-acceptance")
    : await authorCandidate(w, acceptanceStagePolicy());
  let acceptanceRepair = FROZEN_REPAIRS.get(acceptance);
  let skeleton = frozen.skeleton && !acceptanceRepair ? await verifyFrozenStage(w, "verify-frozen-skeleton")
    : await authorCandidate(w, skeletonPolicy(acceptanceRepair ? ["The frozen predecessor was repaired; rebind the skeleton while retaining every unaffected task entry."] : []));
  if (FROZEN_REPAIRS.get(skeleton)?.owner === "acceptance") {
    acceptance = skeleton;
    acceptanceRepair = FROZEN_REPAIRS.get(acceptance);
    skeleton = await authorCandidate(w, skeletonPolicy(["The frozen predecessor was repaired; rebind the skeleton while retaining every unaffected task entry."]));
  }
  if (!Array.isArray(skeleton.subjects) || skeleton.subjects.length === 0) {
    throw new Error("frozen skeleton returned zero host-read task subjects");
  }
  if (acceptanceRepair) FROZEN_REPAIRS.set(skeleton, { owner: "skeleton" });
  requireFrozenSubjects(frozen, skeleton);
  return { acceptance, skeleton, changedChecks: acceptanceRepair ? acceptanceRepair.changedChecks : new Set() };
}

function frozenBodyReusable(frozen, skeleton, subject, changedChecks) {
  if (!frozen.bodies.has(subject.fileName)) return false;
  if (!FROZEN_REPAIRS.has(skeleton)) return true;
  const next = SKELETON_TASKS.get(skeleton);
  if (!frozen.tasks || !next || !frozen.tasks.has(subject.taskId) || !next.has(subject.taskId)) return false;
  if (frozen.tasks.get(subject.taskId) !== next.get(subject.taskId)) return false;
  if (changedChecks === null) return false;
  const entry = JSON.parse(next.get(subject.taskId));
  if ((entry.implements || []).some((id) => changedChecks.has(id))) return false;
  return !changedDependencySubjects(frozen.tasks, next).has(subject.taskId);
}

// A changed deliverable contract reaches its consumers even if their own
// frozen entries stayed identical. Implements-only changes do not change an
// unrelated consumer's input contract. Propagate reached contracts in stable
// skeleton order until the finite subject set is unchanged.
function changedDependencySubjects(prior, next) {
  const reached = new Set();
  for (const [id, text] of next || []) {
    const old = prior?.has(id) ? JSON.parse(prior.get(id)) : null;
    if (!old || JSON.stringify(old.deliverable_contracts || []) !== JSON.stringify(JSON.parse(text).deliverable_contracts || [])) reached.add(id);
  }
  for (let changed = true; changed;) {
    changed = false;
    for (const [id, text] of next || []) {
      if (!reached.has(id) && (JSON.parse(text).depends_on || []).some((edge) => reached.has(edge.task_id))) {
        reached.add(id);
        changed = true;
      }
    }
  }
  return reached;
}
