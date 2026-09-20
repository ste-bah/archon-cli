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

async function authorAcceptanceEntries(w, prompt, round, state = { entries: new Map(), retryIds: null }) {
  const criteria = args.acceptanceCriteria;
  if (!criteria || Object.keys(criteria).length === 0) throw new Error("host acceptanceCriteria are missing");
  const ids = Object.keys(criteria).sort();
  const pending = ids.filter(id => !state.entries.has(id) || state.retryIds === null || state.retryIds.has(id));
  const cap = authorBatchSize();
  for (let start = 0; start < pending.length; start += cap) {
    const batch = pending.slice(start, start + cap);
    const prior = ids.filter(id => state.entries.has(id) && !pending.includes(id))
      .concat(pending.slice(0, start)).map(id => state.entries.get(id));
    const results = await Promise.all(batch.map(async id => {
      for (let retry = 1; retry <= ACCEPTANCE_ATTEMPTS; retry++) {
        const result = await w.agent(`acceptance-author-${id}-${round * ACCEPTANCE_ATTEMPTS + retry}`, {
          task: `${prompt}\nAuthor ONLY entry ${id}: ${criteria[id]}\nAll criterion IDs and text (for consistency): ${JSON.stringify(criteria)}\nPreviously completed entries: ${JSON.stringify(prior)}`,
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
    }));
    // All started calls settle before return; never abandon a sibling agent.
    for (const result of results) if (result.entry) state.entries.set(result.entry.id, result.entry);
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
  return {status:"accepted",stopReason:"end_turn",content:JSON.stringify({entries:ids.map(id => state.entries.get(id))})};
}

// Pre-judge validation can reject a candidate before any receipt exists. Its
// check-local diagnostic still identifies which entries to repair; preserving
// siblings here is not acceptance credit. The full gate runs again afterwards.
function acceptanceRepairIds(findings, knownIds, published) {
  const retry = new Set();
  for (const finding of findings) {
    if (published && knownIds.has(finding.subject)) {
      retry.add(finding.subject);
      continue;
    }
    const text = String(finding.text || "")
      .replace(/^candidate artifact was refused:\s*/, "")
      .replace(/^candidate artifact rejected:\s*/, "");
    if (!/^check '[^']+'(?::| floor | has | judgment )/.test(text)) return null;
    const matches = [...text.matchAll(/(?:^|;\s*|\n)check '([^']+)'/g)];
    if (matches.length === 0 || matches.some(match => !knownIds.has(match[1]))) return null;
    for (const match of matches) retry.add(match[1]);
  }
  return retry;
}
