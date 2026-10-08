// Bounded author context for the fixed decomposition script (Issue 288).
// Loaded after workflow_decompose_v1.js as one script: every declaration
// here is hoisted into the same scope as `workflow`.
//
// An author prompt used to repeat earlier work in full. A live acceptance
// author prompt reached 455 KB, 436 KB of it the 66 entries already
// completed, and every completed entry made every later prompt larger.
// Now everything a prompt repeats from earlier work -- the earlier findings,
// the completed entries and the criterion catalogue -- shares one UTF-8 byte
// budget, CONTEXT_BUDGET, in fixed shares, whatever the number of entries or
// attempts. An earlier result is a one-line record that names the SHA-256 of
// its exact bytes. The host writes those bytes once, to a file named by that
// digest (`__archonAuthorContext`), and the prompt says where to Read them.
// When the records do not fit their share, a deterministic subset is shown,
// the rest is counted, and the complete list is one more file. The current
// findings are never cut: they are what the author repairs. A prompt whose
// own text still passes AUTHOR_PROMPT_LIMIT is not sent: the loop pauses and
// says why (requireDispatchable).

const HISTORY_SHARE = 8192;
const PRIOR_SHARE = 14336;
const CATALOGUE_SHARE = 10240;
const CONTEXT_BUDGET = HISTORY_SHARE + PRIOR_SHARE + CATALOGUE_SHARE;
// The most bytes of one record line (an entry, a criterion) and of one
// earlier finding. A cut line ends in CUT_MARK.
const RECORD_BYTES = 320;
const HISTORY_RECORD_BYTES = 600;
const CUT_MARK = " [cut]";
const AUTHOR_PROMPT_LIMIT = 262144;

function utf8Bytes(text) {
  let bytes = 0;
  for (const char of String(text)) {
    const code = char.codePointAt(0);
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return bytes;
}

// `text` cut to at most `max` UTF-8 bytes, CUT_MARK included, at a character
// boundary. Text that fits is returned unchanged.
function clipBytes(text, max) {
  const value = String(text);
  if (utf8Bytes(value) <= max) return value;
  const room = max - utf8Bytes(CUT_MARK);
  let out = "";
  let used = 0;
  for (const char of value) {
    const size = utf8Bytes(char);
    if (used + size > room) break;
    out += char;
    used += size;
  }
  return out + CUT_MARK;
}

// The longest prefix of `lines` whose list form ("\n- " before each line)
// fits `budget` bytes.
function fitLines(lines, budget) {
  const shown = [];
  let used = 0;
  for (const line of lines) {
    const cost = utf8Bytes(line) + 3;
    if (used + cost > budget) break;
    shown.push(line);
    used += cost;
  }
  return shown;
}

// The file the host wrote `text` to, as {path, sha256}. A missing binding is
// a host fault and throws. A write the host could not make throws an error
// marked `authorContext`, which the caller reports as an outage of the call
// (it pauses the loop when its window closes), never as a reference to bytes
// that are not there.
const CONTEXT_FILES = new Map();
function contextFile(extension, text) {
  const key = `${extension}\n${text}`;
  const known = CONTEXT_FILES.get(key);
  if (known) return known;
  if (typeof __archonAuthorContext !== "function") throw new Error("the host author-context binding is missing");
  let file;
  try {
    file = JSON.parse(__archonAuthorContext(extension, text));
  } catch (error) {
    const failure = new Error(`author context could not be written: ${error && error.message ? error.message : error}`);
    failure.authorContext = true;
    throw failure;
  }
  if (!file || typeof file.path !== "string" || !/^[0-9a-f]{64}$/.test(String(file.sha256))) {
    throw new Error("the host author-context binding returned no file");
  }
  CONTEXT_FILES.set(key, file);
  return file;
}

function contextDir(file) {
  const cut = file.path.lastIndexOf("/");
  return cut < 0 ? "." : file.path.slice(0, cut);
}

// `items` nearest `id` first, by position in `order` (the sorted entry ids);
// ties go to the earlier position. Deterministic in its inputs alone.
function nearestFirst(items, idOf, id, order) {
  const at = (value) => { const index = order.indexOf(value); return index < 0 ? order.length : index; };
  const here = at(id);
  return items.slice().sort((a, b) => Math.abs(at(idOf(a)) - here) - Math.abs(at(idOf(b)) - here) || at(idOf(a)) - at(idOf(b)));
}

function oneLine(value) {
  return String(value === undefined || value === null ? "" : value).replace(/\s+/g, " ").trim();
}

function checkSummary(check) {
  const value = check && typeof check === "object" && !Array.isArray(check) ? check : {};
  if (value.kind === "floor") {
    const contract = value.contract && typeof value.contract === "object" ? value.contract : {};
    const fields = Array.isArray(contract.required_true_fields) ? contract.required_true_fields.join(",") : "";
    return `floor ${oneLine(contract.kind)} ${oneLine(contract.artifact_path)} ${oneLine(contract.artifact_format)} true=[${fields}]: ${oneLine(contract.typed_verifier_command)}`;
  }
  return `${oneLine(value.kind) || "check"} cwd=${oneLine(value.cwd)}: ${oneLine(value.command)}`;
}

// Issue 288: the completed entries an entry author keeps consistent with.
// Each is one record line; its exact JSON is a host-written file.
function priorText(prior, id, order) {
  if (prior.length === 0) return "Previously completed entries: none.";
  const records = prior.map((entry, index) => {
    const file = contextFile("json", JSON.stringify(entry));
    const covers = Array.isArray(entry.covers) ? entry.covers.join(",") : "";
    return { id: entry.id, index, file, line: clipBytes(`${entry.id} sha256:${file.sha256} covers=[${covers}] ${checkSummary(entry.check)}`, RECORD_BYTES) };
  });
  const ranked = nearestFirst(records, (record) => record.id, id, order);
  const count = fitLines(ranked.map((record) => record.line), PRIOR_SHARE).length;
  // Listed in the order they were completed: earlier rounds first.
  const shown = ranked.slice(0, count).sort((a, b) => a.index - b.index);
  let text = `Previously completed entries: ${prior.length}, one line each with its id, the sha256 of its exact JSON, what it covers and a short summary of its check. Keep this entry consistent with their checks -- the flags, paths, keys and formats they rely on -- and duplicate none. A summary is not the check: before you rely on an entry, Read its exact JSON at ${contextDir(records[0].file)}/<its sha256>.json (host-written context for this call: read these files by their exact path, although they sit under an excluded directory).`;
  if (count < records.length) {
    const index = contextFile("jsonl", records.map((record) => `${JSON.stringify({ id: record.id, sha256: record.file.sha256, path: record.file.path })}\n`).join(""));
    text += ` The ${count} nearest this entry are listed; ${records.length - count} more are not, and every one is listed in ${index.path}.`;
  }
  return `${text}\n- ${shown.map((record) => record.line).join("\n- ")}`;
}

// The criterion catalogue, whole while it fits its share; otherwise the
// criteria nearest this entry, one line each, and the exact catalogue file.
function catalogueText(criteria, id, order) {
  const whole = JSON.stringify(criteria);
  const head = "All criterion IDs and text (for consistency):";
  if (utf8Bytes(whole) <= CATALOGUE_SHARE) return `${head} ${whole}`;
  const ids = nearestFirst(Object.keys(criteria).sort(), (value) => value, id, order);
  const count = fitLines(ids.map((value) => clipBytes(`${value}: ${oneLine(criteria[value])}`, RECORD_BYTES)), CATALOGUE_SHARE).length;
  const shown = ids.slice(0, count).sort();
  const file = contextFile("json", whole);
  return `${head} ${ids.length} criteria; the ${count} nearest this entry are listed, one line each, and the exact catalogue is ${file.path} (sha256 ${file.sha256}).\n- ${shown.map((value) => clipBytes(`${value}: ${oneLine(criteria[value])}`, RECORD_BYTES)).join("\n- ")}`;
}

// Whitespace only: findings that differ by case, a sign, a digit or
// punctuation are different findings (a path or a flag can differ by case).
function historyKey(text) {
  return String(text).replace(/\s+/g, " ").trim();
}

// Each distinct earlier finding once, with how often and when it occurred.
// The findings the phase was opened with (attempt 0) come first; a finding
// of the current attempt that earlier attempts also triggered is reported as
// a repeat (`repeats`), not listed again.
function earlierFindings(history, feedback) {
  const current = new Set((feedback || []).map(historyKey));
  const seeded = [];
  const distinct = new Map();
  const repeats = new Map();
  const entries = Array.isArray(history) ? history.slice() : [];
  // The attempt the current findings came from is not an earlier attempt.
  const newest = entries[entries.length - 1];
  if (newest && newest.attempt !== 0 && JSON.stringify((newest.findings || []).map(historyKey)) === JSON.stringify([...(feedback || [])].map(historyKey))) entries.pop();
  for (const entry of entries) {
    for (const text of entry.findings || []) {
      const key = historyKey(text);
      if (entry.attempt === 0) {
        if (!seeded.some((seed) => seed.key === key)) seeded.push({ key, text: String(text), count: 1, first: 0, last: 0 });
        continue;
      }
      const table = current.has(key) ? repeats : distinct;
      const known = table.get(key);
      if (known) { known.count += 1; known.last = entry.attempt; } else table.set(key, { text: String(text), count: 1, first: entry.attempt, last: entry.attempt });
    }
  }
  const seedKeys = new Set(seeded.map((seed) => seed.key));
  const others = [...distinct.entries()].filter(([key]) => !seedKeys.has(key)).map(([, found]) => found)
    .sort((a, b) => b.last - a.last || b.count - a.count || a.first - b.first);
  return { seeded, others, repeats };
}

function authorPrompt(base, attempt, feedback, history) {
  if (feedback.length === 0) return `${base}\nLogical attempt: ${attempt}.`;
  const earlier = earlierFindings(history, feedback);
  const span = (found) => (found.first === found.last ? `attempt ${found.last}` : `attempts ${found.first}-${found.last}`);
  const repeat = (text) => {
    const seen = earlier.repeats.get(historyKey(text));
    return seen ? ` (a repeat: seen in ${seen.count} earlier attempt${seen.count === 1 ? "" : "s"}, ${span(seen)})` : "";
  };
  let prompt = `${base}\nLogical attempt: ${attempt}. Repair these exact authoritative findings:\n- ${feedback.map((text) => `${text}${repeat(text)}`).join("\n- ")}`;
  // Two gates can be individually satisfiable and jointly hard. Without the
  // history an author repairs the finding in front of it, trips the other, and
  // alternates until its budget is spent -- a live acceptance phase did exactly
  // that for all six attempts. Showing what earlier attempts already triggered
  // is what lets it satisfy both at once instead of trading one for the other.
  const records = [...earlier.seeded, ...earlier.others];
  const full = records.map((found) => (found.first === 0 ? `the set gate, before this body was sent back: ${found.text}`
    : `${found.count === 1 ? `attempt ${found.last}` : `attempts ${found.first}-${found.last}, ${found.count} times`}: ${found.text}`));
  const lines = fitLines(full.map((line) => clipBytes(line, HISTORY_RECORD_BYTES)), HISTORY_SHARE);
  if (lines.length > 0) {
    prompt += `\nEarlier attempts in this phase already triggered the following. Satisfy every one of them at once; repairing the finding above by reverting an earlier repair will not converge:\n- ${lines.join("\n- ")}`;
  }
  const hidden = records.slice(lines.length);
  if (hidden.length > 0) {
    const attempts = hidden.flatMap((found) => [found.first, found.last]);
    prompt += `\n${hidden.length} older distinct findings (${hidden.reduce((sum, found) => sum + found.count, 0)} occurrences, attempts ${Math.min(...attempts)}-${Math.max(...attempts)}) are not repeated here.`;
  }
  if (hidden.length > 0 || lines.some((line, index) => line !== full[index])) {
    const file = contextFile("json", JSON.stringify(records.map(({ text, count, first, last }) => ({ text, count, first, last }))));
    prompt += `\nEvery earlier finding, in full with its count and attempts, is in ${file.path} (host-written context for this call: read it by its exact path).`;
  }
  return prompt;
}

// A prompt whose own text passes AUTHOR_PROMPT_LIMIT is never sent: its
// base and current findings cannot be cut without hiding what the author
// repairs, so the loop pauses with the measured size and the remedy, and a
// resumed loop that still measures it pauses again.
async function requireDispatchable(w, subject, task) {
  for (;;) {
    const bytes = utf8Bytes(task);
    if (bytes <= AUTHOR_PROMPT_LIMIT) return task;
    await pauseLoop(w, subject, {
      reason: "author_prompt_oversized", prompt_bytes: bytes, limit_bytes: AUTHOR_PROMPT_LIMIT, context_budget_bytes: CONTEXT_BUDGET,
      diagnosis: "the prompt's mandatory text (the phase instructions and the current findings) alone exceeds the dispatch limit; the earlier-work context is already bounded",
      remedy: "resuming alone pauses again: reduce the current findings this call must repair (split the input that produces them), or raise AUTHOR_PROMPT_LIMIT in the fixed decomposition script (a code change), then resume"
    });
  }
}
