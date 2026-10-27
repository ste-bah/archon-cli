// Acceptance-entry authoring helpers for the fixed decomposition script.
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.

// Issue 366: one rule for both authors, held equal by test; absolute live roots are refused unrun.
// Apply it in every configuration, including live runs without isolation.
// The proof and entry validator refuse absolute live roots before any check runs.
function checkPathRule() {
  return [
    "Where a check runs: a check starts in a POSIX shell whose working directory is the project root when cwd is project_root (a floor's typed_verifier_command always starts there) or the repository root when cwd is repo_root, of whichever tree the host runs it in. Before acceptance the host proves each check in a disposable copy: a clone of the code repository at its committed HEAD (uncommitted changes are not in it) and the project files the host is configured to copy, which may be fewer than the project holds. An acceptance round runs the same check in such a copy or, when no isolated acceptance execution is configured, in the live repository and project themselves. So a check must not change, delete or reset anything outside its own temporary files (build output its own commands produce excepted). Choose the cwd whose root holds the files the command reads: repo_root for repository source and tests; project_root may reach repository files via ../<repository dir>/ when the repository directory sits beside the project and the check names an existing path in that tree (the site then links the repository copy there); repo_root names repository paths relative to repo_root and cannot use `..` to reach project_root or its own repository. If a check passes before implementation, data it writes itself is never evidence; movable inputs must exist under its selected root before the run. To guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name it relative to that root. The check's pass or fail must be decided by the existing test or implementation it names, and it must fail whenever that test fails. Naming a file or asserting that it exists does not make it the deciding assertion. A status-discarding sequence, a run whose failure cannot fail the check, or matching text in a source or test file does not establish the criterion. The move-aside proof applies only to inputs the host actually moves: the host may prove the guard by moving such a literally named input aside and requiring the check to fail, and that proof is never a counterexample; the check's own script, a script run through an interpreter, a directory it changes into and a build manifest are never moved and do not qualify.",
    `In every check command, name every path relative to the check's working directory; never write the repository's or the project's absolute path (${args.repositoryRoot}, ${args.projectRoot}) or any path under them: those paths are for your reading only. The host refuses a check that names either root by its absolute path and never runs it.`
  ].join("\n");
}

// The per-entry acceptance author's prompt (supplementary checks included).
function acceptanceAuthorPrompt() {
  return [
    "Author exactly one acceptance entry identified below, not the whole contract.",
    `Read the PRD at ${args.prdPath}.`,
    groundingRules(),
    "Use the repository only to verify real test names and paths; every path or test the entry names must be one you observed under the repository root.",
    checkPathRule(),
    "A named deciding input that is absent is behaviour not implemented yet and is judged by the general rules; a named deciding input whose content is not supplied has unknown assertions, so the check cannot be accepted as a regression guard on assumed content.",
    "Reply format: see the end of this task.",
    ENTRY_SHAPES,
    "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges every entry and validates the assembled contract together.",
    "Every check must fail on the pre-implementation baseline when its criterion is false; when the criterion already holds on that baseline, it must still be able to fail after the host moves aside every data file it names by a path relative to its working directory (never its own script, a program it runs, a directory it changes into or a build manifest), so make it read the files that decide that criterion by those paths.",
    "Use the exact supplied id. Criterion and judgment are host-owned placeholders. Set gap_permitted only if the PRD permits that criterion to remain a documented gap.",
    "covers lists every requirement id the PRD defines (REQ-*) whose violation, on the path this check drives, makes the check fail; list none the check would still pass under. Every PRD requirement must be covered by some check: the host names each one no check covers as a supplementary check SUP-<requirement id> it is owed, which you then author like an entry, covering exactly that requirement.",
    "For a command check, choose EXACTLY ONE command form. Block form: the JSON check object holds kind, cwd and \"command_block\": true, with NO \"command\" field, followed by exactly one non-empty fence whose info string is exactly 'check'. Inline form: the JSON check object holds kind, cwd and \"command\" only; command is one short line without quotes or backslashes. Include no command_block and no fence.",
    "For a multi-line script, use block form: set \"command_block\": true in the entry and put the script unescaped in exactly one fence labelled check after the JSON entry; then no escaping is needed.",
    "Do not run commands or write files."
  ].join("\n");
}

function acceptanceReplyFormat() {
  return [
    "Return exactly one complete reply with one entry and one check.",
    "A complete reply looks exactly like this (each fence line alone on its line):",
    "```json",
    "{\"id\":\"<ID>\",\"criterion\":\"...\",\"check\":{\"kind\":\"command\",\"command_block\":true,\"cwd\":\"project_root\"},\"gap_permitted\":false,\"judgment\":{\"verdict\":\"accepted\",\"counterexample\":\"\",\"reason\":\"\",\"host_call_id\":\"\"}}",
    "```",
    "```check",
    "set -eu",
    "printf '%s\\n' 'run the focused check'",
    "make test",
    "```",
    "",
    "Every fence line is alone on its line: open with exactly ```json or ```check, close with exactly ```; never put text after a fence on the same line.",
    "Choose EXACTLY ONE command form. Block form: the JSON check object holds kind, cwd and \"command_block\": true and NO \"command\" field, followed by exactly one non-empty fence whose info string is exactly 'check'. Inline form: the JSON check object holds kind, cwd and \"command\" only; command is one short line without quotes or backslashes. Include no command_block and no fence. Never write an empty check block.",
    "For a multi-line script, use block form: set \"command_block\": true in the entry and put the script unescaped in exactly one fence labelled check after the JSON entry; then no escaping is needed.",
    "Generate large or repetitive fixture data with code rather than writing it literally.",
    "The entry shapes earlier in this task are examples only; return one entry.",
    "Return exactly the entry: for inline form, output only the JSON entry (raw or in one `json` fence); for block form, output the JSON entry (raw or in one `json` fence) followed by exactly one non-empty fence labelled `check`. Emit no prose, explanations, headings or other fences."
  ].join("\n");
}

// The live roots a check must not name, as the validator reads them: the
// roots this prompt names, and the task root the host reads the freeze's own
// roots from (its configured scratch repository), so both steps refuse a
// check from one source.
function liveRootsText() {
  return JSON.stringify({repository: args.repositoryRoot, project: args.projectRoot, tasks: args.taskRoot});
}

// Completed entries survive a sibling's incomplete reply. The host validates
// each authored entry with freeze's shape validator; freeze assembles and
// judges the full contract. Neither step trusts model-side validation.
// The reply SHOULD be a bare JSON object; sometimes it is fenced or preceded by
// prose. A bare JSON.parse turns that formatting slip into a reply without
// progress, and enough of them stop the entry while every reply carried a
// usable object. A live run died exactly that way: one acceptance entry
// "exhausted 6 replies" when three were ```json-fenced objects and three were
// prose that ended in one.
//
// Take the outermost {...}. Anything that still fails to parse is genuinely
// malformed and retries as before.
// The author was shown ACCEPTANCE_SHAPE -- the whole contract -- as the example
// of what an entry looks like, and told not to return the enclosing contract.
// It returned the enclosing contract anyway: a live run produced
// { schema_version, prd, gap_policy, acceptance: [ <the right entry> ] } on
// four consecutive attempts for one entry, each holding exactly the entry
// asked for, each rejected because the top-level object had no id. Same shape
// of failure as the fence bug: usable content, wrong envelope, spent attempt.
//
// If the reply is a contract whose acceptance list holds exactly one entry
// with the requested id, that entry is the answer.
function unwrapEntry(parsed, id) {
  if (!parsed || typeof parsed !== "object") return parsed;
  if (parsed.id === id) return parsed;
  const list = Array.isArray(parsed.acceptance) ? parsed.acceptance : null;
  if (list && list.length === 1 && list[0] && typeof list[0] === "object") return list[0];
  return parsed;
}

// H4 (Batch O): every PRD requirement id is covered by some check. The host
// freeze names each requirement no entry covers as the supplementary check it
// is owed ("check 'SUP-<id>': PRD requirement <id> is covered by no
// acceptance check; ...: <requirement text>"); acceptanceRepairIds records it
// on itself (`acceptanceRepairIds.owed`, so it stays one self-contained
// declaration), and the next attempt authors exactly those, beside the entries.
function owedSupplementary() {
  if (!acceptanceRepairIds.owed) acceptanceRepairIds.owed = new Map();
  return acceptanceRepairIds.owed;
}

// The host owns an entry's criterion: freeze stamps the exact acceptance
// criterion over it, and a supplementary check's requirement text
// (workflow_acceptance_preflight.rs). A supplementary check also always
// covers exactly the requirement it is owed for and never permits a gap. The
// host sets these fields before the entry is validated, so the validator
// judges the entry that is kept and a value the host overwrites never costs a
// repair (Issue 357). Every authored id is a criterion or an owed check, so
// the host value is always known here.
function setHostOwnedFields(entry, criteria) {
  const sup = owedSupplementary().get(entry.id);
  if (Object.prototype.hasOwnProperty.call(criteria, entry.id)) entry.criterion = criteria[entry.id];
  else if (sup) entry.criterion = currentRequirementText(sup.requirement);
  if (sup) {
    const covers = Array.isArray(entry.covers) ? entry.covers.filter((c) => typeof c === "string") : [];
    entry.covers = [sup.requirement, ...covers.filter((c) => c !== sup.requirement)];
    entry.gap_permitted = false;
  }
  return entry;
}

// Each entry makes one provider call per round. The phase records the round's
// retained work and failure; no inner retry window can hide malformed replies
// or restart its count when a transport error interrupts them. Keep the call
// id stride so replay of already-recorded rounds uses the same namespace.
async function authorOne(w, prompt, round, id, text, prior, criteria, state) {
  // Each entry reads only its own shape refusal: a sibling's names another
  // entry and is not this entry's repair.
  const refused = state.refusals && state.refusals.get(id);
  const own = (refused ? `\nThe host refused this entry's last answered reply. Repair exactly these findings:\n- ${refused.join("\n- ")}` : "")
    + supersededDraftText(state, id);
  // Issue 288: the catalogue and the completed entries are bounded records
  // that name host-written files (workflow_decompose_v1_context.js). A file
  // the host could not write is an outage of this entry's call, made before
  // any provider call, never a prompt that names bytes that are not there.
  let task;
  try {
    task = `${prompt}\nAuthor ONLY entry ${id}: ${text}${own}\n${catalogueText(criteria, id, state.order)}\n${priorText(prior, id, state.order)}\n${acceptanceReplyFormat()}`;
    task = namedFilesContext(task, state.entries?.get(id) || {id, criterion:text}, [text, own, ...currentAuthorFindings(prompt)]);
  } catch (error) {
    if (!error || error.authorContext !== true) throw error;
    return {failure:redactFailure({status:"failed",summary:`acceptance entry ${id}: ${error.message}`})};
  }
  task = await requireDispatchable(w, `acceptance-prompt-${id}`, task);
  state.roundCalls += 1;
  const result = await w.agent(`acceptance-author-${id}-${round * STALL_ATTEMPTS + 1}`, {
    task, tier: "planner", resultMode: "rawOutcome"
  });
  if (result.status !== "failed") state.roundAnswered += 1;
  if (result.dry_run === true) return {entry:setHostOwnedFields({id}, criteria)};
  if (result.status === "failed") return {failure:redactFailure(result)};
  if (result.stopReason !== "end_turn") {
    if (result.stopReason === "max_tokens") {
      return malformedAcceptanceFailure(id,
        incompleteReplyFeedback(result.content),
        "stop-max-tokens");
    }
    return malformedAcceptanceFailure(id, `expected stop reason end_turn, got ${result.stopReason || "missing"}`,
      stopReasonClass(result.stopReason));
  }
  if (typeof result.content !== "string" || result.content.length === 0) {
    return malformedAcceptanceFailure(id, "stop reason was end_turn but content was missing or empty", "missing-content");
  }
  {
    let entry;
    let json;
    try {
      json = "\n".repeat(fencedEntryLinePrefix(result.content, args.replyLeakedMarkers))
        + extractJsonObject(result.content, args.replyLeakedMarkers);
      entry = unwrapEntry(JSON.parse(json), id);
    } catch (error) {
      if (error && / is never closed \(the reply may have been cut off\)/.test(error.message)) {
        return malformedAcceptanceFailure(id, error.message, "unclosed-fence");
      }
      if (error && error.message === "reply contains more than one entry") {
        return malformedAcceptanceFailure(id, error.message, "multiple-entries");
      }
      if (error && error.multipleJsonInvalid === true) {
        return malformedAcceptanceFailure(id, redactSecretShapedText(error.message), parseErrorClass(error));
      }
      return malformedAcceptanceFailure(id, acceptanceParseRefusal(id, json, error), parseErrorClass(error));
    }
    const blockRefusal = resolveCommandBlock(entry, result.content, args.replyLeakedMarkers);
    if (blockRefusal) return malformedAcceptanceFailure(id, blockRefusal, "command-block-refusal");
    if (entry && entry.id === id) {
      setHostOwnedFields(entry, criteria);
      let serialized;
      // QuickJS bounds JSON.stringify by its stack as it bounds JSON.parse: a
      // reply nested past it is no entry, like one that does not parse.
      try { serialized = JSON.stringify(entry); } catch (_) { /* no progress */ }
      if (serialized !== undefined) {
        // Native binding uses freeze's element_shape_defects with one entry;
        // text it cannot parse (an unpaired surrogate, too deep) is its
        // invalid_json refusal. A missing binding or a validator fault must
        // propagate, never accept. Each text names the entry by its id. The
        // live roots let it refuse a check naming one, as freeze does (366).
        const defects = JSON.parse(__archonValidateAcceptanceEntry(id, serialized, liveRootsText())).map(redactFinding);
        if (defects.length === 0) return {entry};
        return {failure:{status:"failed",malformed:true,findings:defects,entryId:id,
          summary:defects.map(defect => defect.text).join("; ")}};
      }
    }
    if (entry && Object.prototype.hasOwnProperty.call(entry, "id")) {
      return malformedAcceptanceFailure(id, `expected id ${id}, got id ${JSON.stringify(entry.id)}`, "wrong-id");
    }
  }
  return malformedAcceptanceFailure(id, `reply did not contain a complete entry with id ${id}`, "missing-entry");
}

// Parsed JSON objects have no meaningful property order. Formatting changes
// cannot turn an unchanged completed entry into progress.
function acceptanceEntryKey(entry) {
  const ordered = (value) => {
    if (Array.isArray(value)) return value.map(ordered);
    if (!value || typeof value !== "object") return value;
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])]));
  };
  return JSON.stringify(ordered(entry));
}

async function authorAcceptanceEntries(w, prompt, round, state = { entries: new Map(), retryIds: null }) {
  state.roundCalls = 0;
  state.roundAnswered = 0;
  state.added = state.added || 0;
  state.replaced = state.replaced || 0;
  const criteria = args.acceptanceCriteria;
  if (!criteria || Object.keys(criteria).length === 0) throw new Error("host acceptanceCriteria are missing");
  const ids = Object.keys(criteria).sort();
  // A supplementary check is authored like an entry, against the one
  // requirement it is owed for, and always covers exactly that requirement.
  const owedMap = owedSupplementary();
  const owed = [...owedMap.keys()].sort();
  const textOf = (id) => {
    if (Object.prototype.hasOwnProperty.call(criteria, id)) return criteria[id];
    const sup = owedMap.get(id);
    return `SUPPLEMENTARY check owed to PRD requirement ${sup.requirement} (no other check covers it): ${sup.text}\nIts covers is exactly ["${sup.requirement}"]; it must fail whenever ${sup.requirement} is violated on the path it drives. Like every check, it starts from its working directory, which may be in a copy or in the live checkout: name every path relative to the check's working directory, never by the repository's or the project's absolute path, and change nothing outside its own temporary files.`;
  };
  const all = ids.concat(owed);
  state.order = all;
  const pending = all.filter(id => !state.entries.has(id) || state.retryIds === null || state.retryIds.has(id));
  const cap = authorBatchSize();
  const earlier = all.filter(id => state.entries.has(id) && !pending.includes(id)).map(id => state.entries.get(id));
  // Prefix window (Issue-247): entry i starts once entries 0..i-cap of this
  // round have settled, and sees exactly those (in order) after the entries of
  // earlier rounds. Its prompt depends on its index alone, so a resume reuses
  // the recorded call, and a slow entry delays only the entries behind it.
  const { settled } = await runBounded(pending.length, cap, true, (index, done) => {
    const prior = earlier.concat(done.slice(0, Math.max(0, index - cap + 1)).map(result => result.value.entry));
    return authorOne(w, prompt, round, pending[index], textOf(pending[index]), prior, criteria, state);
  }, (result) => Boolean(result && result.failure));
  // Every started call has settled. The window starts an index only while its
  // fixed prefix holds no failure, so which entries ran depends on the replies
  // alone, never on timing (runBounded). Every entry that succeeded is kept,
  // in input order, the ones after a failure too: re-authoring a passed entry
  // wastes its call and can bring back a finding it already repaired.
  const succeeded = (result) => Boolean(result && result.status === "fulfilled" && !result.value.failure);
  const stop = settled.findIndex(result => !succeeded(result));
  for (const result of settled.filter(succeeded)) {
    const entry = result.value.entry;
    if (entry) {
      // A new entry is a new best; a changed rewrite of one the judge sent
      // back is only novelty until the judge accepts it (Issue 261).
      if (!state.entries.has(entry.id)) state.added += 1;
      else if (acceptanceEntryKey(state.entries.get(entry.id)) !== acceptanceEntryKey(entry)) state.replaced += 1;
      recordDraft(state, entry.id, entry);
      state.entries.set(entry.id, entry);
    }
  }
  // Every refusal of the round is returned and measured, each against its own
  // entry's best, and each entry is shown only its own refusal next round.
  const refusals = settled.filter(result => result && result.status === "fulfilled" && result.value.failure?.findings)
    .map(result => ({entryId:result.value.failure.entryId, findings:result.value.failure.findings,
      malformedClass:result.value.failure.malformedClass}));
  state.roundPassed = pending.filter((_, index) => succeeded(settled[index]));
  // Each entry's slot holds the note of its last answered reply: a shape
  // refusal, or the note of a reply that held no complete entry. A pass
  // clears it; a call never answered leaves it, since that reply's refusal is
  // still the one to repair.
  state.refusals = state.refusals || new Map();
  pending.forEach((id, index) => {
    const result = settled[index];
    if (!result || result.status !== "fulfilled") return;
    const failure = result.value.failure;
    if (failure && failure.entryId === id && Array.isArray(failure.findings)) state.refusals.set(id, failure.findings.map(progressText));
    else if (failure && failure.malformed) state.refusals.set(id, [failure.summary]);
    else if (!failure) state.refusals.delete(id);
  });
  // Provider/context failures are operational outages. A validator refusal
  // has findings and remains an author attempt instead.
  state.roundOutage = pending.some((_, index) => {
    const failure = settled[index]?.status === "fulfilled" ? settled[index].value.failure : null;
    return Boolean(failure) && failure.malformed !== true && !Array.isArray(failure.findings);
  });
  // A thrown call (a pause or cancel the host observed, or a host error)
  // outranks a failed reply at any index: returned as a failed value it would
  // be retried as an operational failure and the stop would be lost.
  const rejected = settled.find(result => result && result.status === "rejected");
  if (rejected) throw rejected.reason;
  // Issue 362: a clean round leaves nothing to re-author. Until the gate
  // names entries again, a freeze retried after an operational outage reuses
  // these entries and makes no author call.
  if (stop < 0) state.retryIds = new Set();
  if (stop < 0) return {status:"accepted",stopReason:"end_turn",content:JSON.stringify({
    entries: ids.map(id => state.entries.get(id)),
    supplementary: owed.map(id => state.entries.get(id)).filter(Boolean)
  })};
  const first = settled[stop];
  // Unreachable unless the pool stopped without a failure: fail loudly.
  if (!first) throw new Error(`acceptance entry ${pending[stop]} was never authored`);
  // The failures and the indices never started are retried.
  state.retryIds = new Set(pending.filter((_, index) => !succeeded(settled[index])));
  if (refusals.length === 0) return redactFailure(first.value.failure);
  const findings = refusals.flatMap(refusal => refusal.findings);
  return {status:"failed", malformed:true, refusals, findings, summary:findings.map(progressText).join("; ")};
}

// Pre-judge validation can reject a candidate before any receipt exists. Its
// check-local diagnostic still identifies which entries to repair; preserving
// siblings here is not acceptance credit. The full gate runs again afterwards.
function acceptanceRepairIds(findings, knownIds, published, candidate, heldEntries) {
  const retry = new Set();
  // A supplementary check the host says is owed is known from then on.
  const supFinding = /^check '(SUP-(REQ-[A-Za-z0-9-]+))': PRD requirement \2 is covered by no acceptance check;[^:]*:\s*([\s\S]*)$/;
  if (!acceptanceRepairIds.owed) acceptanceRepairIds.owed = new Map();
  const owed = acceptanceRepairIds.owed;
  const heldOmissions = new Set();
  for (const finding of findings) {
    const match = supFinding.exec(String(finding.text || ""));
    const defect = finding && finding.deterministic_defect;
    const defectId = defect?.code === "uncovered_requirement" && typeof defect.subject === "string"
      ? /^SUP-(REQ-[A-Za-z0-9-]+)$/.exec(defect.subject)
      : null;
    const omissionSubject = defect?.code === "uncovered_requirement"
      && typeof defect.subject === "string" ? defect.subject : null;
    const id = match?.[1] || omissionSubject;
    const requirement = match?.[2] || (defectId ? defectId[1] : null);
    if (id && requirement) owed.set(id, { requirement, text: currentRequirementText(requirement) });
    if (id && heldEntries && typeof heldEntries.has === "function" && heldEntries.has(id)) {
      heldOmissions.add(id);
      continue;
    }
    if (!id || !requirement) continue;
    retry.add(id);
  }
  const known = { has: (id) => knownIds.has(id) || owed.has(id) };
  for (const finding of findings) {
    const defect = finding && finding.deterministic_defect;
    if (defect?.code === "uncovered_requirement" && heldOmissions.has(defect.subject)) continue;
    if (supFinding.test(String(finding.text || ""))
      || (defect?.code === "uncovered_requirement" && /^SUP-REQ-[A-Za-z0-9-]+$/.test(defect.subject || ""))) continue;
    if (published && known.has(finding.subject)) {
      retry.add(finding.subject);
      continue;
    }
    const text = String(finding.text || "")
      .replace(/^candidate artifact was refused:\s*/, "")
      .replace(/^candidate artifact rejected:\s*/, "");
    // Resolve against the submitted arrays, not sorted ids: supplementary
    // entries have their own indices, and neither array is an id namespace.
    const pointer = /^\/?(entries|supplementary)\/(0|[1-9]\d*)(?:\/[^\s]*)?(?=\s|$)/.exec(text);
    if (pointer) {
      const list = candidate?.[pointer[1]];
      const id = Array.isArray(list) ? list[Number(pointer[2])]?.id : undefined;
      if (typeof id !== "string" || !known.has(id)) return null;
      retry.add(id);
      continue;
    }
    // A refuted check is never published (the host stages the envelope alone),
    // so its "was refuted" finding names the one entry to re-author.
    // A check that crashed, or passed before any implementation (the host's
    // executability probe, Batch O), names its one entry too.
    if (!/^check '[^']+'(?::| floor | has | judgment | was refuted | crashed )/.test(text)) return null;
    const matches = [...text.matchAll(/(?:^|;\s*|\n)check '([^']+)'/g)];
    if (matches.length === 0 || matches.some(match => !known.has(match[1]))) return null;
    for (const match of matches) retry.add(match[1]);
  }
  return retry;
}

// A staged freeze whose contract changed after it was prepared publishes
// nothing and returns the current published contract
// (`report.current_acceptance_contract`, null when none is published). Its
// entries replace this loop's, so a newer published repair is never
// overwritten by an older candidate, and its supplementary checks are owed
// from then on. A finding of this
// outcome judged the older candidate, so it sends back only an id the
// current contract does not supply; that, or an id with no entry, is
// re-authored. Returns the rebuilt candidate when nothing is, else null with
// `state.retryIds` set for the next author round.
function refreshFromCurrentContract(state, report, others, ids, candidate) {
  if (!report || !Object.prototype.hasOwnProperty.call(report, "current_acceptance_contract")) {
    throw new Error("a stale acceptance freeze returned no current contract to rebuild from");
  }
  const current = report.current_acceptance_contract === null
    ? { acceptance: [], supplementary: [] } : report.current_acceptance_contract;
  if (!current || !Array.isArray(current.acceptance) || !Array.isArray(current.supplementary)) {
    throw new Error("a stale acceptance freeze returned a current contract without entry lists");
  }
  const owed = owedSupplementary();
  for (const entry of current.supplementary) {
    const sup = /^SUP-(REQ-[A-Za-z0-9-]+)$/.exec(String(entry && entry.id));
    if (sup && !owed.has(entry.id)) owed.set(entry.id, { requirement: sup[1], text: currentRequirementText(sup[1]) });
  }
  const supplied = new Set();
  const drafts = state.drafts || new Map();
  for (const entry of current.acceptance.concat(current.supplementary)) {
    if (!entry || typeof entry.id !== "string") continue;
    supplied.add(entry.id);
    const draft = drafts.get(entry.id);
    // This loop's unpublished draft is newer than the published entry it was
    // drafted from. A published entry that changed since that base is newer
    // still: it wins, and the draft is shown to the entry's next author.
    if (draft && draft.base && authoredEntryKey(draft.base) === authoredEntryKey(entry)) continue;
    if (draft) {
      state.superseded = state.superseded || new Map();
      state.superseded.set(entry.id, draft.draft);
      drafts.delete(entry.id);
    }
    state.entries.set(entry.id, entry);
  }
  const all = [...ids].sort().concat([...owed.keys()].sort());
  const attributed = acceptanceRepairIds(others, ids, false, candidate, state.entries);
  const flagged = attributed === null ? null : new Set([...attributed].filter((id) => !supplied.has(id)));
  const missing = all.filter((id) => !state.entries.has(id));
  if (flagged !== null && flagged.size === 0 && missing.length === 0) {
    state.retryIds = new Set();
    return JSON.stringify({
      entries: [...ids].sort().map((id) => state.entries.get(id)),
      supplementary: [...owed.keys()].sort().map((id) => state.entries.get(id))
    });
  }
  state.retryIds = flagged === null ? null : new Set([...flagged, ...missing]);
  return null;
}

// An entry's authored fields: what the author wrote, without the judgment
// the host adds when it publishes it (the host's `authored_entry_digest`).
function authoredEntryKey(entry) {
  return acceptanceEntryKey({ id: entry.id, criterion: entry.criterion, check: entry.check,
    covers: Array.isArray(entry.covers) ? entry.covers : [], gap_permitted: entry.gap_permitted === true });
}

// This loop's drafts not yet published, each with the entry it replaced:
// the base a stale freeze's current contract is compared with. A publication
// clears them (`clearPublishedDrafts`).
function recordDraft(state, id, entry) {
  state.drafts = state.drafts || new Map();
  if (!state.drafts.has(id)) state.drafts.set(id, { base: state.entries.get(id) || null });
  state.drafts.get(id).draft = entry;
}

function clearPublishedDrafts(state) {
  state.drafts = new Map();
}

// The draft a newer published entry replaced, shown once to its next author.
function supersededDraftText(state, id) {
  const draft = state.superseded && state.superseded.get(id);
  if (!draft) return "";
  state.superseded.delete(id);
  return `\nA newer published version of this entry replaced your previous draft before it was published. Your previous draft was:\n${JSON.stringify(draft)}`;
}

function currentRequirementText(requirement) {
  const text = (args.prdRequirementTexts || {})[requirement];
  if (typeof text !== "string") throw new Error(`current PRD text is missing for supplementary requirement ${requirement}`);
  return text;
}
