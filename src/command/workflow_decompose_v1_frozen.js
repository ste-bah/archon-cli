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
        if (tasks) {
          SKELETON_TASKS.set(outcome, tasks);
          CURRENT_SKELETON_TASKS = tasks;
        }
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

function skeletonObligationContext() {
  if (!Array.isArray(args.prdObligationIds)) return "Use only PRD obligation ids extracted by the host.";
  const ids = JSON.stringify(args.prdObligationIds);
  return ids.length <= 4096
    ? `Exact PRD obligation ids extracted by the host: ${ids}`
    : `Exact PRD obligation ids extracted by the host: Read the JSON array at ${contextFile("json", ids).path} (host-written context for this call: read it by its exact path).`;
}

// The host supplies the map; authors assign applicability, never re-extract ids.
function constraintContext(author = "skeleton") {
  const texts = args.prdConstraintTexts || {};
  if (Object.keys(texts).length === 0) return [];
  const json = JSON.stringify(texts);
  const listed = json.length <= 4096 ? json
    : `Read the JSON object at ${contextFile("json", json).path} (host-written context for this call: read it by its exact path).`;
  const rule = author === "skeleton"
    ? "Every host-listed constraint id is in the implements of at least one task. Each host-listed PRD constraint applies to the tasks whose work it restricts (all tasks if it restricts all work); name each applicable constraint id in the task's implements."
    : author === "body"
      ? "Read the host-listed PRD constraint texts; this task's work must keep every listed constraint its frozen implements names."
      : null;
  return [...(rule ? [rule] : []),
    `Exact PRD constraint text extracted by the host: ${listed}`];
}

// Exact section texts use the same host-written context mechanism as constraints.
// An empty section map adds no context bytes; the skeleton shape still declares
// source_sections so future PRD sections have a frozen carrier.
function sectionContext(subject) {
  const sections = args.prdSections || {};
  if (Object.keys(sections).length === 0) return [];
  if (!subject) {
    const listed = Object.entries(sections).filter(([, section]) => section.requires_citation)
      .map(([number, section]) => `${JSON.stringify(number)}: ${section.title}`).join("\n");
    const compact = utf8Bytes(listed) <= 4096 ? listed
      : `Read ${contextFile("txt", listed).path} (host-written context for this call: read it by its exact path).`;
    return ["Cite sections by the key the host lists (a section number, or the exact heading title of an unnumbered top-level section). For every host-listed qualifying PRD section, cite it in at least one task: every task whose work it justifies, or any one task when it justifies none (metadata, guidance or closing statements); a task cites every section its work comes from. Parent-only headings need no separate citation when their sub-sections are cited.",
      `Qualifying PRD sections (key: title):\n${compact}`,
      `Full PRD section texts: Read ${contextFile("json", JSON.stringify(sections)).path} (host-written context for this call: read it by its exact path).`];
  }
  const entry = CURRENT_SKELETON_TASKS?.get(subject.taskId) || frozenChain().tasks?.get(subject.taskId);
  // Host outcomes supply the current frozen entry after a repair, not the
  // launch snapshot. Older hosts expose only the task root: read it explicitly.
  if (!entry) return ["Read this task's frozen source_sections in task-skeleton.json; the task body must carry out what those sections require for this task's work."];
  const numbers = JSON.parse(entry).source_sections || [];
  const cited = Object.fromEntries(numbers.filter(number => sections[number]).map(number => [number, sections[number]]));
  return ["The task body must carry out what its frozen source_sections require for this task's work.",
    `Frozen source_sections: ${JSON.stringify(numbers)}`,
    `Cited PRD section texts: Read ${contextFile("json", JSON.stringify(cited)).path} (host-written context for this call: read it by its exact path).`];
}

// The skeleton's author policy, whether it authors the skeleton first or
// re-authors it because the set gate found an obligation no task claims or
// a claim the PRD does not define (Batch O). `initialFeedback` carries the
// set gate's exact findings into the first attempt.
const SKELETON_AUTHOR_MARKER = "Land task records for the frozen acceptance contract.";

function skeletonPolicy(initialFeedback, initialRung, previousCandidate, directive = "") {
  return {
    phase: "skeleton",
    previousCandidate,
    capability: "freeze-skeleton",
    retryScopes: new Set(["candidate_artifact", "skeleton"]),
    initialFeedback,
    directive,
    ...(Number.isInteger(initialRung) ? { initialRung } : {}),
    prompt: (previous) => [
      SKELETON_AUTHOR_MARKER,
      ...(directive ? [directive] : []),
      `Read the PRD at ${args.prdPath}, the task root at ${args.taskRoot}, and the repository source you need.`,
      groundingRules(),
      "Stop reading once you can name what your entries assert.",
      "Every artifact_path is a path relative to the repository root. Observe under the repository root whether each one exists before you declare it, and give every repository file or directory the PRD names by path an owning task: the host refuses a skeleton that leaves one unowned.",
      "Every PRD obligation id extracted by the host is in the implements of at least one task, and implements names only host-listed obligation or constraint ids. Obligations are requirement bullets, rows of tables whose header names an obligation, and numbered done-definition items (synthetic ids). The host also lists PRD constraints stated as list items or table rows that restrict work. When the task root already holds a frozen skeleton, keep every task_id and file_name it freezes: repair an obligation no task claims by adding its id to the implements of the task that owns that work (or by adding a task), never by removing a task.",
      skeletonObligationContext(),
      ...constraintContext(),
      ...sectionContext(),
      "Each landed task must deserialize into this shape:",
      JSON.stringify(JSON.parse(SKELETON_SHAPE).tasks[0]),
      "Every <...> above is a placeholder describing the value, never a value: replace each one.",
      "One task per unit of work; task_id and file_name become the frozen tuple the bodies must preserve.",
      "Write task_id as the unquoted canonical TASK-<AREA>-<NNN>; its value must equal the TASK-<AREA>-<NNN> id in the filename, without the extension.",
      "depends_on and blocks are empty arrays when the task has no such relation; every entry present takes exactly the shape shown.",
      "Each depends_on entry declares a non-empty consumes list or ordering_only: true.",
      "The consumes record-binding fields are optional: omit unused fields. For a multi-writer artifact_path, include the same record-binding field and exact value as the named producer's deliverable contract, or use a producer-specific artifact_path. Use instance_source_records_field when the producer declares it; otherwise use registry_records_field. These values name the producer's records field, not a task id or artifact path.",
      deliverableContractRule(),
      "The deliverable_contracts record-binding fields are optional: omit unused fields; a producer of a multi-writer artifact that dependents bind by records declares the same records field and exact value in its deliverable contract as in the dependents' consumes bindings.",
      "Generate large or repetitive fixture data with code rather than writing it literally.",
      "Land each task with the landing tool; your final reply is exactly {\"records_landed\":N}, with N from the last landing receipt. N counts new landings in this call, excluding seeds. A call with open findings must make at least one byte-changing landing; a call with an empty task store must land at least one task. The gate then judges the whole set again.",
      "Do not run commands or write files."
    ].join("\n") + previousSkeletonContext(previous)
  };
}

// #568: carry exact candidate bytes through the existing host-written context
// binding. No content means no extra line, including on a first attempt.
function previousSkeletonContext(content) {
  if (typeof content !== "string" || content.length === 0) return "";
  const file = contextFile("json", content);
  return `\nThe previous candidate is in ${file.path} (sha256 ${file.sha256}; host-written context for this call: read it by its exact path).`
    + "\nWhatever you restructure, keep every task, implements id, source_sections entry, deliverable and edge of the previous candidate that no current finding names. Compare the Landed subjects list at the end of this prompt with the previous candidate before landing changes and re-land every missing task with valid fields, preserving its work unless a current finding requires its removal. The host can drop invalid seeds. Only the subjects listed there have been retained. Re-land changed tasks. "
    + (frozenChain().skeleton || CURRENT_SKELETON_TASKS !== null ? "Keep every frozen task_id and file_name." : "Use remove:true to drop a draft task.");
}

// #554: the operational findings of a freeze outcome whose subject is an
// entry this loop authored. `repeated` holds those whose entry is unchanged
// since the previous freeze reported it unproven: retrying the host did not
// help, so its author is told. Each is remembered for the next freeze.
function unprovenAuthored(state, outcome, routed) {
  const findings = routed.operational
    ? (Array.isArray(outcome?.gateEnvelope?.policy_findings) ? outcome.gateEnvelope.policy_findings : [])
    : routed.fatalFindings;
  const seen = new Map();
  const repeated = new Set();
  for (const finding of findings) {
    if (finding?.remediation_scope !== "operational" || typeof finding.subject !== "string") continue;
    if (!state.entries || !state.entries.has(finding.subject)) continue;
    const key = acceptanceEntryKey(state.entries.get(finding.subject));
    seen.set(finding.subject, key);
    if (state.unproven?.get(finding.subject) === key) repeated.add(finding);
  }
  state.unproven = seen;
  return { repeated, subjects: new Set(seen.keys()) };
}

// #555: the entries each gate finding is for, read as the gate's routing
// reads it: its subject when that is a criterion, else acceptanceRepairIds.
// An omission of a held supplementary check is that held entry's. A finding
// no rule attributes to one entry is phase-level. An entry's prompt shows
// its own findings and the phase-level ones, never a sibling's. A stale
// contract is the loop's to rebuild (refreshFromCurrentContract), no author's.
function attributeEntryFindings(findings, ids, published, candidate, held) {
  const own = new Map();
  const phase = [];
  for (const finding of findings) {
    const selected = ids.has(finding.subject) ? new Set([finding.subject])
      : acceptanceRepairIds([finding], ids, published, candidate, held);
    const holder = [finding.subject, finding.deterministic_defect?.subject].find((id) => typeof id === "string" && held?.has(id));
    const owners = selected && selected.size === 0 && holder ? new Set([holder]) : selected;
    if (owners === null && finding?.deterministic_defect?.code !== "stale_acceptance_contract") {
      const text = String(finding.text || "unnamed policy finding");
      if (!phase.includes(text)) phase.push(text);
    }
    for (const id of owners || []) {
      const open = own.get(id) || [];
      pushGateFinding(open, finding, id);
      own.set(id, open);
    }
  }
  return { own, phase };
}

// #555 review: the phase-level findings of the latest gate are current; an
// earlier one it no longer reports is kept as one not to regress, and one it
// reports again is current again, as settleGateFindings keeps an entry's by defect identity.
function settlePhaseFindings(state, latest) {
  const earlier = (state.earlierPhaseFindings || []).slice();
  for (const text of state.phaseFindings || []) if (!latest.includes(text) && !earlier.includes(text)) earlier.push(text);
  state.earlierPhaseFindings = earlier.filter((text) => !latest.includes(text));
  state.phaseFindings = latest;
}

// #555 review r2: an entry's earlier findings (its own, or the phase-level
// ones), bounded as authorPrompt bounds the phase history: the newest that
// fit HISTORY_SHARE, each clipped to HISTORY_RECORD_BYTES, and the whole list,
// which holds no sibling's finding, in a host-written file read by its path.
function earlierFindingsText(heading, texts) {
  if (!Array.isArray(texts) || texts.length === 0) return "";
  const full = texts.map(String);
  const count = fitLines(full.slice().reverse().map((found) => clipBytes(found, HISTORY_RECORD_BYTES)), HISTORY_SHARE).length;
  const shown = full.slice(full.length - count).map((found) => clipBytes(found, HISTORY_RECORD_BYTES));
  let out = `\n${heading}${shown.length > 0 ? `\n- ${shown.join("\n- ")}` : ""}`;
  if (count < full.length) out += `\n${full.length - count} older distinct findings are not repeated here.`;
  if (count < full.length || shown.some((line, index) => line !== full[full.length - count + index])) {
    out += `\nEvery one, in full, is in ${contextFile("json", JSON.stringify(full)).path} (host-written context for this call: read it by its exact path).`;
  }
  return out;
}

async function repairFrozenStage(w, verified, findings) {
  const owner = frozenFindingOwner(findings[0]);
  if (findings.some((finding) => frozenFindingOwner(finding) !== owner)) {
    // Verification of one stage reports one owning artifact. Keep all mixed
    // ownership findings pending by repairing the predecessor first.
    return await repairFrozenStage(w, verified, findings.filter((finding) => frozenFindingOwner(finding) === "acceptance"));
  }
  if (owner === "skeleton") {
    const repaired = await authorCandidate(w, skeletonPolicy(findings.map(findingText), undefined, skeletonCandidateContent(verified)));
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
      // #555: each entry starts with only its own verification findings.
      const opened = attributeEntryFindings(findings, new Set(Object.keys(args.acceptanceCriteria || {})), true, contract, state.entries);
      [state.gateFindings, state.phaseFindings, state.earlierPhaseFindings] = [opened.own, opened.phase, []];
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
    : await authorCandidate(w, skeletonPolicy([], undefined, undefined, acceptanceRepair ? "The frozen predecessor was repaired; rebind the skeleton while retaining every unaffected task entry." : ""));
  if (FROZEN_REPAIRS.get(skeleton)?.owner === "acceptance") {
    acceptance = skeleton;
    acceptanceRepair = FROZEN_REPAIRS.get(acceptance);
    skeleton = await authorCandidate(w, skeletonPolicy([], undefined, undefined, "The frozen predecessor was repaired; rebind the skeleton while retaining every unaffected task entry."));
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

function malformedAcceptanceFailure(id, reason, malformedClass) {
  const finding = {text:redactSecretShapedText(`candidate artifact was refused: acceptance entry ${id}: ${String(reason)}`), subject:id};
  return {failure:{status:"failed",malformed:true,malformedClass,entryId:id,findings:[finding],summary:finding.text}};
}

// Show retained coverage so a rewrite makes each removal deliberate.
function currentCoversText(entry) {
  return Array.isArray(entry?.covers) && entry.covers.length > 0
    ? `\nThis entry currently covers ${JSON.stringify(entry.covers)}. Keep every one of them in covers unless the new check no longer fails when that requirement is violated; the host re-opens each one you drop that no other check covers as an owed supplementary check.`
    : "";
}

// Omission preserves the current entry; an explicit value remains the author's.
// Call after host-owned fields so supplementary coverage still belongs to the host.
function inheritAcceptanceCovers(entry, prior) {
  if (!Object.prototype.hasOwnProperty.call(entry, "covers") && Array.isArray(prior?.covers)) {
    entry.covers = prior.covers.slice();
  }
}

// Verified frozen stages can carry host-read entries without an authored candidate.
function skeletonLandingSeed(previous) {
  if (parsedSkeletonCandidate(previous)) return previous;
  const tasks = CURRENT_SKELETON_TASKS || frozenChain().tasks;
  return tasks ? JSON.stringify({ schema_version: 1, acceptance_digest: "", tasks: [...tasks.values()].map(text => JSON.parse(text)) }) : null;
}

// Candidate-artifact refusals never become setup inputs for the next author.
function parsedSkeletonCandidate(content) {
  try { return Array.isArray(JSON.parse(content).tasks); } catch (_) { return false; }
}
function acceptedSkeletonCandidate(content, outcome) {
  return parsedSkeletonCandidate(content) && !!outcome?.gateEnvelope
    && !outcome.gateEnvelope.operational_error
    && !(outcome.gateEnvelope.policy_findings || []).some(f => f.remediation_scope === "candidate_artifact");
}

function skeletonPreviousCandidate(policy, previous, seeded) {
  if (policy.phase !== "skeleton") return previous;
  const seed = args.phaseSeed?.subjects?.skeleton;
  if (seeded && seed?.candidate === previous && (!seed?.gate || ((seed.gate.findings || []).length > 0
    && seed.gate.findings.every(f => f.remediation_scope === "candidate_artifact")))) return undefined;
  return parsedSkeletonCandidate(previous) ? previous : undefined;
}

// C6: raw dispatch composes a zero-landing receipt into an empty or seeded
// artifact. Empty stores and open findings require work; durable seed replay bypasses this check.
function refuseUnchangedSkeleton(authored, carry, openFindings = []) {
  if (carry !== null || authored.status === "failed" ||
      authored.stopReason !== "end_turn" || authored.retainedLandings === true) return authored;
  let tasks;
  try { tasks = JSON.parse(authored.content).tasks; } catch (_) { return authored; }
  if (!Array.isArray(tasks)) return authored;
  const empty = tasks.length === 0;
  if (!empty && openFindings.length === 0) return authored;
  return { ...authored, status: "failed", invalid_answer: true,
    summary: openFindings.length > 0 ? "This call has open findings and made no byte-changing landing. Land at least one new or changed task record before replying; the gate judges the retained set." : "The task store is empty: land at least one task before replying." };
}
