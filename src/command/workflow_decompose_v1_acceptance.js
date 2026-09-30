// Acceptance-entry authoring helpers for the fixed decomposition script.
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.

// Completed entries survive a sibling's incomplete reply. Assembly and validation
// belong to freeze-acceptance, not to a model or an unchecked JSON concatenation.
// The reply SHOULD be a bare JSON object; sometimes it is fenced or preceded by
// prose. A bare JSON.parse turns that formatting slip into a spent attempt, and
// with ACCEPTANCE_ATTEMPTS of them one entry can exhaust the whole budget while
// every reply carried a usable object. Run wf-cddf8426 died exactly that way:
// AC-AHDM-001 "exhausted 6 replies" when three were ```json-fenced objects and
// three were prose that ended in one.
//
// Take the outermost {...}. Anything that still fails to parse is genuinely
// malformed and retries as before.
// The author was shown ACCEPTANCE_SHAPE -- the whole contract -- as the example
// of what an entry looks like, and told not to return the enclosing contract.
// It returned the enclosing contract anyway: run wf-379a1faa produced
// { schema_version, prd, gap_policy, acceptance: [ <the right entry> ] } on
// four consecutive attempts for AC-AHDM-002, each holding exactly the entry
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

async function authorOne(w, prompt, round, id, text, prior, criteria) {
  for (let retry = 1; retry <= ACCEPTANCE_ATTEMPTS; retry++) {
    const result = await w.agent(`acceptance-author-${id}-${round * ACCEPTANCE_ATTEMPTS + retry}`, {
      task: `${prompt}\nAuthor ONLY entry ${id}: ${text}\nAll criterion IDs and text (for consistency): ${JSON.stringify(criteria)}\nPreviously completed entries: ${JSON.stringify(prior)}`,
      tier: "planner", resultMode: "rawOutcome"
    });
    if (result.dry_run === true) return {entry:{id}};
    if (result.status === "failed") return {failure:result};
    if (result.stopReason !== "end_turn" || !result.content) continue;
    try {
      const entry = unwrapEntry(JSON.parse(extractJsonObject(result.content)), id);
      if (entry && entry.id === id) return {entry};
    } catch (_) { /* Retry only this malformed entry. */ }
  }
  return {failure:{status:"failed",summary:`acceptance entry ${id} exhausted ${ACCEPTANCE_ATTEMPTS} replies`}};
}

async function authorAcceptanceEntries(w, prompt, round, state = { entries: new Map(), retryIds: null }) {
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
  for (let start = 0; start < pending.length; start += cap) {
    const batch = pending.slice(start, start + cap);
    const prior = all.filter(id => state.entries.has(id) && !pending.includes(id))
      .concat(pending.slice(0, start)).map(id => state.entries.get(id));
    const results = await Promise.all(batch.map(id => authorOne(w, prompt, round, id, textOf(id), prior, criteria)));
    // All started calls settle before return; never abandon a sibling agent.
    for (const result of results) {
      if (!result.entry) continue;
      const sup = owedMap.get(result.entry.id);
      if (sup) {
        const covers = Array.isArray(result.entry.covers) ? result.entry.covers.filter((c) => typeof c === "string") : [];
        result.entry.covers = [sup.requirement, ...covers.filter((c) => c !== sup.requirement)];
        result.entry.gap_permitted = false;
      }
      state.entries.set(result.entry.id, result.entry);
    }
    const failure = results.find(result => result.failure);
    if (failure) {
      state.retryIds = new Set(pending.filter(id => !state.entries.has(id)));
      for (let index = 0; index < results.length; index++) {
        if (results[index].failure) state.retryIds.add(batch[index]);
      }
      for (const id of pending.slice(start + batch.length)) state.retryIds.add(id);
      return failure.failure;
    }
  }
  return {status:"accepted",stopReason:"end_turn",content:JSON.stringify({
    entries: ids.map(id => state.entries.get(id)),
    supplementary: owed.map(id => state.entries.get(id)).filter(Boolean)
  })};
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
