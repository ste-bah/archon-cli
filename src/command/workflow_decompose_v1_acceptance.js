// Acceptance-entry authoring helpers for the fixed decomposition script.
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.

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
  if (list && list.length === 1 && list[0] && list[0].id === id) return list[0];
  return parsed;
}

function extractJsonObject(text) {
  const raw = String(text || "").trim();
  const fenced = raw.match(/```(?:json)?\s*([\s\S]*?)```/);
  const body = (fenced ? fenced[1] : raw).trim();
  const first = body.indexOf("{");
  const last = body.lastIndexOf("}");
  return first >= 0 && last > first ? body.slice(first, last + 1) : body;
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
  else if (sup) entry.criterion = sup.text;
  if (sup) {
    const covers = Array.isArray(entry.covers) ? entry.covers.filter((c) => typeof c === "string") : [];
    entry.covers = [sup.requirement, ...covers.filter((c) => c !== sup.requirement)];
    entry.gap_permitted = false;
  }
  return entry;
}

// Issue 288: the author sees each completed entry without its host-owned
// criterion and judgment (the criteria are listed separately; the judgment is
// host text) -- one JSON line per entry holding its id, covers and its whole
// check. The check is what later entries must agree with: the CLI flags,
// output paths, JSON keys and formats it relies on. PRIOR_ENTRY_TEXT guards
// only against a runaway entry (real checks run to about 16 KB); past it the
// line is cut and the cut is marked. The short fields are written before the
// commands, so a cut can only shorten a command.
const PRIOR_ENTRY_TEXT = 65536;

function commandsLast(check) {
  if (!check || typeof check !== "object" || Array.isArray(check)) return check;
  const { command, contract, ...rest } = check;
  let ordered = contract;
  if (contract && typeof contract === "object" && !Array.isArray(contract)) {
    const { typed_verifier_command, ...short } = contract;
    ordered = { ...short, typed_verifier_command };
  }
  return { ...rest, contract: ordered, command };
}

function priorEntry(entry) {
  const value = entry && typeof entry === "object" ? entry : {};
  const text = JSON.stringify({ id: value.id, covers: value.covers, gap_permitted: value.gap_permitted, check: commandsLast(value.check) });
  if (text.length <= PRIOR_ENTRY_TEXT) return text;
  return `${text.slice(0, PRIOR_ENTRY_TEXT)} [CUT: ${text.length - PRIOR_ENTRY_TEXT} more characters of this entry are not shown]`;
}

function priorText(prior) {
  if (prior.length === 0) return "Previously completed entries: none.";
  return `Previously completed entries, one JSON line each without the host-owned criterion and judgment (keep this entry consistent with their checks -- the flags, paths, keys and formats they rely on -- and duplicate none):\n- ${prior.map(priorEntry).join("\n- ")}`;
}

// Each entry makes one provider call per round. The phase records the round's
// retained work and failure; no inner retry window can hide malformed replies
// or restart its count when a transport error interrupts them. Keep the call
// id stride so replay of already-recorded rounds uses the same namespace.
async function authorOne(w, prompt, round, id, text, prior, criteria, state) {
  state.roundCalls += 1;
  // Each entry reads only its own shape refusal: a sibling's names another
  // entry and is not this entry's repair.
  const refused = state.refusals && state.refusals.get(id);
  const own = refused ? `\nThe host refused this entry's last answered reply. Repair exactly these findings:\n- ${refused.join("\n- ")}` : "";
  const result = await w.agent(`acceptance-author-${id}-${round * STALL_ATTEMPTS + 1}`, {
    task: `${prompt}\nAuthor ONLY entry ${id}: ${text}${own}\nAll criterion IDs and text (for consistency): ${JSON.stringify(criteria)}\n${priorText(prior)}`,
    tier: "planner", resultMode: "rawOutcome"
  });
  if (result.status !== "failed") state.roundAnswered += 1;
  if (result.dry_run === true) return {entry:setHostOwnedFields({id}, criteria)};
  if (result.status === "failed") return {failure:result};
  if (result.stopReason === "end_turn" && result.content) {
    let entry;
    try {
      entry = unwrapEntry(JSON.parse(extractJsonObject(result.content)), id);
    } catch (_) { /* This answered call made no progress. */ }
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
        // propagate, never accept. Each text names the entry by its id.
        const defects = JSON.parse(__archonValidateAcceptanceEntry(id, serialized));
        if (defects.length === 0) return {entry};
        return {failure:{status:"failed",malformed:true,findings:defects,entryId:id,
          summary:defects.map(defect => defect.text).join("; ")}};
      }
    }
  }
  return {failure:{status:"failed",malformed:true,summary:`acceptance entry ${id} returned no complete entry`}};
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
    return `SUPPLEMENTARY check owed to PRD requirement ${sup.requirement} (no other check covers it): ${sup.text}\nIts covers is exactly ["${sup.requirement}"]; it must fail whenever ${sup.requirement} is violated on the path it drives.`;
  };
  const all = ids.concat(owed);
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
      state.entries.set(entry.id, entry);
    }
  }
  // Every refusal of the round is returned and measured, each against its own
  // entry's best, and each entry is shown only its own refusal next round.
  const refusals = settled.filter(result => result && result.status === "fulfilled" && result.value.failure?.findings)
    .map(result => ({entryId:result.value.failure.entryId, findings:result.value.failure.findings}));
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
  // A call that failed in transport is an outage of the round, whatever the
  // first failure is (refused or unparseable replies hide no outage).
  state.roundOutage = pending.some((_, index) => {
    const failure = settled[index]?.status === "fulfilled" ? settled[index].value.failure : null;
    return Boolean(failure) && failure.malformed !== true;
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
  if (refusals.length === 0) return first.value.failure;
  const findings = refusals.flatMap(refusal => refusal.findings);
  return {status:"failed", malformed:true, refusals, findings, summary:findings.map(progressText).join("; ")};
}

// Pre-judge validation can reject a candidate before any receipt exists. Its
// check-local diagnostic still identifies which entries to repair; preserving
// siblings here is not acceptance credit. The full gate runs again afterwards.
function acceptanceRepairIds(findings, knownIds, published, candidate) {
  const retry = new Set();
  // A supplementary check the host says is owed is known from then on.
  const supFinding = /^check '(SUP-(REQ-[A-Za-z0-9-]+))': PRD requirement \2 is covered by no acceptance check;[^:]*:\s*([\s\S]*)$/;
  if (!acceptanceRepairIds.owed) acceptanceRepairIds.owed = new Map();
  const owed = acceptanceRepairIds.owed;
  for (const finding of findings) {
    const match = supFinding.exec(String(finding.text || ""));
    if (!match) continue;
    owed.set(match[1], { requirement: match[2], text: match[3].trim() });
    retry.add(match[1]);
  }
  const known = { has: (id) => knownIds.has(id) || owed.has(id) };
  for (const finding of findings) {
    if (supFinding.test(String(finding.text || ""))) continue;
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
