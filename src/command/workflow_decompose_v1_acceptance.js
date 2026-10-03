// Acceptance-entry authoring helpers for the fixed decomposition script.
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.

// Completed entries survive a sibling's incomplete reply. Assembly and validation
// belong to freeze-acceptance, not to a model or an unchecked JSON concatenation.
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

// Each entry makes one provider call per round. The phase records the round's
// retained work and failure; no inner retry window can hide malformed replies
// or restart its count when a transport error interrupts them. Keep the call
// id stride so replay of already-recorded rounds uses the same namespace.
async function authorOne(w, prompt, round, id, text, prior, criteria, state) {
  state.roundCalls += 1;
  const result = await w.agent(`acceptance-author-${id}-${round * STALL_ATTEMPTS + 1}`, {
    task: `${prompt}\nAuthor ONLY entry ${id}: ${text}\nAll criterion IDs and text (for consistency): ${JSON.stringify(criteria)}\nPreviously completed entries: ${JSON.stringify(prior)}`,
    tier: "planner", resultMode: "rawOutcome"
  });
  if (result.status !== "failed") state.roundAnswered += 1;
  if (result.dry_run === true) return {entry:{id}};
  if (result.status === "failed") return {failure:result};
  if (result.stopReason === "end_turn" && result.content) {
    try {
      const entry = unwrapEntry(JSON.parse(extractJsonObject(result.content)), id);
      if (entry && entry.id === id) return {entry};
    } catch (_) { /* This answered call made no progress. */ }
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
  const owe = (result) => {
    const sup = result.entry && owedMap.get(result.entry.id);
    if (sup) {
      const covers = Array.isArray(result.entry.covers) ? result.entry.covers.filter((c) => typeof c === "string") : [];
      result.entry.covers = [sup.requirement, ...covers.filter((c) => c !== sup.requirement)];
      result.entry.gap_permitted = false;
    }
    return result;
  };
  // Prefix window (Issue-247): entry i starts once entries 0..i-cap of this
  // round have settled, and sees exactly those (in order) after the entries of
  // earlier rounds. Its prompt depends on its index alone, so a resume reuses
  // the recorded call, and a slow entry delays only the entries behind it.
  const { settled } = await runBounded(pending.length, cap, true, (index, done) => {
    const prior = earlier.concat(done.slice(0, Math.max(0, index - cap + 1)).map(result => result.value.entry));
    return authorOne(w, prompt, round, pending[index], textOf(pending[index]), prior, criteria, state).then(owe);
  }, (result) => Boolean(result && result.failure));
  // Every started call has settled. Only the entries before the first entry
  // that did not succeed are kept, in input order: whether a later entry got
  // to start or finish before the failure is a matter of timing, so keeping it
  // would make the next round's prompts and call set depend on timing too.
  const stop = settled.findIndex(result => !result || result.status !== "fulfilled" || result.value.failure);
  const kept = stop < 0 ? settled.length : stop;
  for (const result of settled.slice(0, kept)) {
    const entry = result.value.entry;
    if (entry) {
      // A new entry is a new best; a changed rewrite of one the judge sent
      // back is only novelty until the judge accepts it (Issue 261).
      if (!state.entries.has(entry.id)) state.added += 1;
      else if (acceptanceEntryKey(state.entries.get(entry.id)) !== acceptanceEntryKey(entry)) state.replaced += 1;
      state.entries.set(entry.id, entry);
    }
  }
  // A thrown call (a pause or cancel the host observed, or a host error)
  // outranks a failed reply at any index: returned as a failed value it would
  // be retried as an operational failure and the stop would be lost.
  const rejected = settled.find(result => result && result.status === "rejected");
  if (rejected) throw rejected.reason;
  if (stop < 0) return {status:"accepted",stopReason:"end_turn",content:JSON.stringify({
    entries: ids.map(id => state.entries.get(id)),
    supplementary: owed.map(id => state.entries.get(id)).filter(Boolean)
  })};
  const first = settled[stop];
  // Unreachable unless the pool stopped without a failure: fail loudly.
  if (!first) throw new Error(`acceptance entry ${pending[stop]} was never authored`);
  state.retryIds = new Set(pending.slice(stop));
  return first.value.failure;
}

// Pre-judge validation can reject a candidate before any receipt exists. Its
// check-local diagnostic still identifies which entries to repair; preserving
// siblings here is not acceptance credit. The full gate runs again afterwards.
function acceptanceRepairIds(findings, knownIds, published) {
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
