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
  const value = String(text);
  // Each UTF-16 unit costs 1/2/3 bytes; a valid pair costs 4, not 6.
  return value.length + value.replace(/[^\u0080-\uffff]/g, "").length
    + value.replace(/[^\u0800-\uffff]/g, "").length
    - 2 * (value.match(/[\ud800-\udbff][\udc00-\udfff]/g) || []).length;
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
  if (newest && newest.attempt !== 0 && (newest.findings || []).every((text) => current.has(historyKey(text)))) entries.pop();
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
    prompt += `\nEarlier attempts in this phase raised the following; a finding not in the list above is no longer reported, so it is not one to fix now. Do not regress them: keep every one satisfied at once; repairing a finding above by reverting an earlier repair will not converge:\n- ${lines.join("\n- ")}`;
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

function currentAuthorFindings(prompt) {
  const marker = "Repair these exact authoritative findings:\n- ";
  const start = prompt.indexOf(marker);
  if (start < 0) return [];
  return prompt.slice(start + marker.length)
    .split("\nEarlier attempts in this phase already triggered the following.")[0]
    .split("\n- ");
}

// The supplied files sit before `tail` (the reply format), which stays the
// final task section; the room for them leaves the tail's bytes free.
function namedFilesContext(task, entry, notes, tail = "") {
  const base = `${task}\nAnswer once you have read the supplied named_files content for the deciding file, or inspected that file at its named path yourself.\nFiles already identified for this check are supplied here when available: `;
  if (utf8Bytes(base) + utf8Bytes("[]") + utf8Bytes(tail) > AUTHOR_PROMPT_LIMIT) return `${task}${tail}`;
  const room = Math.max(0, AUTHOR_PROMPT_LIMIT - utf8Bytes(base) - utf8Bytes(tail));
  const supplied = typeof __archonNamedFiles === "function"
    ? JSON.parse(__archonNamedFiles(JSON.stringify(entry), JSON.stringify(notes), room)) : [];
  const files = [];
  let used = 2; // The array brackets; each later element also needs a comma.
  for (const file of supplied) {
    const comma = files.length > 0 ? 1 : 0;
    let shown = JSON.stringify(file);
    let size = utf8Bytes(shown);
    if (used + comma + size > room) {
      shown = JSON.stringify({path:file.path, status:"not_supplied"});
      size = utf8Bytes(shown);
    }
    if (used + comma + size > room) break;
    files.push(shown);
    used += comma + size;
  }
  return `${base}[${files.join(",")}]${tail}`;
}

// Checkpoints use the same journal and watch path as the host's log primitive.
async function repairEvent(w, subject, rung, cycle, call, attempts) {
  if (typeof w.checkpoint !== "function") return;
  await w.checkpoint(`repair-${subject}-r${rung}-c${cycle}-${call}-a${attempts}`, {
    task: `${subject} repair rung r${rung}, cycle ${cycle}, attempts ${attempts}`
  });
}

// The set gate's retry scopes. Every body was judged alone against its own
// claims; a finding that only the whole set reveals (an obligation task A
// claims, hollowed by text in task B) is still a body defect, and Issue-46
// sends it back to the body it names instead of ending the run.
const SET_GATE_RETRY_SCOPES = new Set(["body", "candidate_artifact"]);

function authoredSourceMatches(source, fileName) {
  if (typeof source !== "string" || !source) return false;
  const normalize = (path) => {
    const text = String(path).replace(/\\/g, "/");
    const parts = [];
    for (const part of text.split("/")) {
      if (!part || part === ".") continue;
      if (part === ".." && parts.length && parts[parts.length - 1] !== "..") parts.pop();
      else if (part !== ".." || !text.startsWith("/")) parts.push(part);
    }
    return `${text.startsWith("/") ? "/" : ""}${parts.join("/")}`;
  };
  return normalize(source) === normalize(`${args.taskRoot}/${fileName}`);
}

async function runSetGate(w, capability, subjects) {
  const outcome = await w.hostCommand(capability, { stdin: null });
  // Set-level skeleton findings are routed to a skeleton re-author by the
  // loop (Batch O), never shadowed past.
  const routed = routeFindings(outcome, SET_GATE_RETRY_SCOPES, new Set(["skeleton"]));
  for (const finding of routed.fatalFindings.slice()) {
    const owner = (subjects || []).find((subject) => authoredSourceMatches(finding.source_path, subject.fileName));
    if (!owner) continue;
    routed.fatalFindings = routed.fatalFindings.filter((candidate) => candidate !== finding);
    routed.fatal = routed.fatal.filter((text) => !String(text).includes(String(finding.text || "unnamed policy finding")));
    routed.retry.push(String(finding.text || "unnamed policy finding"));
    routed.retryFindings.push(finding);
  }
  if (routed.fatal.length > 0) {
    await stopFixed(`${capability} stopped: ${routed.fatal.join(" | ")}`);
  }
  return { capability, outcome, routed };
}

// Protocol v1 deliberately changes the embedded script digest for a planned release.
// Only an actual validated host request makes this stop terminal; error text cannot.
async function stopFixed(reason) {
  await __archonHost("terminalStop", JSON.stringify({ schemaVersion: 1, reason: String(reason).slice(0, 4096) }));
  throw new Error("host returned without honoring the terminal stop");
}

// Batch O: a set gate is accepted only with no finding open, in either
// mode. Observe used to accept one with findings and build on it.
function acceptSetGate(gate) {
  if (gate.routed.all.length > 0) {
    throw new Error(`${gate.capability} cannot be accepted with findings open: ${gate.routed.all.join(" | ")}`);
  }
  requireCommitted(gate.outcome, gate.capability);
  return gate.outcome;
}

// A gate operational error (a truncated judge reply, a preparation failure)
// is returned as `operational`, never thrown: the caller's loop counts it as
// an attempt without progress and pauses the run when its window closes.
function routeFindings(outcome, retryScopes, shadowScopes) {
  if (!outcome || typeof outcome !== "object") throw new Error("host command returned no typed outcome");
  const routed = {
    retry: [], retryFindings: [], fatal: [], fatalFindings: [], inherited: [], inheritedFindings: [],
    shadow: [], shadowFindings: [], all: [], allFindings: [], operational: null,
    fatalOperational: []
  };
  if (outcome.gateEnvelope?.operational_error) {
    routed.operational = String(outcome.gateEnvelope.operational_error.text || "host gate operational failure");
    routed.operationalKind = String(outcome.gateEnvelope.operational_error.kind || "");
    return routed;
  }
  const findings = Array.isArray(outcome.gateEnvelope?.policy_findings)
    ? outcome.gateEnvelope.policy_findings
    : [];
  const shadows = shadowScopes || new Set();
  for (const finding of findings) {
    const text = typeof finding.text === "string" ? finding.text : "unnamed policy finding";
    const scope = finding.remediation_scope;
    routed.all.push(text);
    routed.allFindings.push(finding);
    if (scope === "prd_input" || scope === "operational") {
      routed.fatal.push(text);
      routed.fatalFindings.push(finding);
      routed.fatalOperational[routed.fatal.length - 1] = scope === "operational";
    }
    else if (scope === "inherited_predecessor") { routed.inherited.push(text); routed.inheritedFindings.push(finding); }
    else if (retryScopes.has(scope)) { routed.retry.push(text); routed.retryFindings.push(finding); }
    else if (shadows.has(scope)) { routed.shadow.push(text); routed.shadowFindings.push(finding); }
    // Every remaining finding stops the phase. A missing or unrecognised scope
    // is operational by contract, and a scope this phase cannot act on means
    // something the candidate does not own changed underneath it. Falling
    // through here silently left retry and fatal both empty, so the phase
    // accepted the artifact and reported success.
    else {
      routed.fatal.push(`scope '${scope || "missing"}' is not actionable in this phase: ${text}`);
      routed.fatalFindings.push(finding);
      routed.fatalOperational[routed.fatal.length - 1] = false;
    }
  }
  return routed;
}

function requireCommitted(outcome, phase) {
  if (!outcome.publicationReceipt) throw new Error(`${phase} returned no committed publication receipt`);
  if (!outcome.postcondition || outcome.postcondition.satisfied !== true) {
    throw new Error(`${phase} authoritative postcondition is not satisfied`);
  }
}

// `authored` is the number of body outcomes this run carries: acceptance,
// skeleton, one per subject this run authored or re-authored, and the two set
// gates. A body the frozen chain supplied and the set gates never sent back
// has no author outcome; the two committed set-gate outcomes, whose
// postcondition compares every task on disk to the frozen skeleton, are its
// evidence.
function reconcile(evidence, authored) {
  if (!Array.isArray(evidence) || evidence.length !== authored + 4) {
    throw new Error("Phase E evidence cardinality does not match frozen subjects");
  }
  for (const [index, outcome] of evidence.entries()) {
    requireCommitted(outcome, `Phase E evidence ${index + 1}`);
    const receiptId = outcome.publicationReceipt.call_id;
    const recordedId = outcome.result?.data?.publicationReceipt?.call_id;
    if (typeof receiptId !== "string" || receiptId.length === 0 || receiptId !== recordedId) {
      throw new Error(`Phase E receipt identity mismatch at evidence ${index + 1}`);
    }
  }
}

function requireSubject(subject) {
  if (!subject || typeof subject.taskId !== "string" || typeof subject.fileName !== "string") {
    throw new Error("frozen skeleton returned a malformed host-read subject");
  }
}

// Describe only patterns visible in the incomplete reply. Character counts
// use Unicode code points, and empty content has no lines.
function incompleteReplyFeedback(content) {
  const text = typeof content === "string" ? content : "";
  const characters = Array.from(text).length;
  const lines = text === "" ? [] : text.split(/\r\n|\r|\n/);
  const groups = new Map();
  const fieldType = (value) => {
    const field = value.trim();
    if (field === "") return "empty";
    if (/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(field)) return "number";
    if (/^(?:true|false|null)$/i.test(field)) return field.toLowerCase();
    return "text";
  };
  for (const line of lines) {
    const trimmed = line.trim();
    let key = null;
    let kind = null;
    if (trimmed.startsWith("{") && trimmed.endsWith("}")) {
      try {
        const value = JSON.parse(trimmed);
        if (value && typeof value === "object" && !Array.isArray(value)) {
          const fields = Object.keys(value).sort().map((name) => {
            const item = value[name];
            const type = item === null ? "null" : Array.isArray(item) ? "array" : typeof item;
            return `${name}:${type}`;
          }).join(",");
          key = `json:${fields}`;
          kind = "one-object-per-line JSON";
        }
      } catch (_) { /* A malformed line is not a JSON data row. */ }
    }
    if (key === null) {
      for (const delimiter of [",", "\t"]) {
        const fields = line.split(delimiter);
        if (fields.length < 2) continue;
        key = `delimited:${delimiter}:${fields.length - 1}:${fields.map(fieldType).join(":")}`;
        kind = delimiter === "," ? "CSV" : "TSV";
        break;
      }
    }
    if (key !== null) {
      const group = groups.get(key) || {count:0, kind};
      group.count++;
      groups.set(key, group);
    }
  }
  let largest = null;
  for (const group of groups.values()) if (!largest || group.count > largest.count) largest = group;
  const size = `The provider cut off an incomplete reply of ${characters} characters across ${lines.length} lines`;
  if (largest && largest.count > lines.length / 2) {
    return `${size}; ${largest.count} of ${lines.length} lines share the same literal ${largest.kind} shape. Generate large or repetitive fixture data inside the check with a loop or small program, and keep only what the criterion needs.`;
  }
  return `${size}; return a shorter complete artifact.`;
}


// Where the authors read code (Issue-55). The repository root is the ONLY
// place source paths, test names, module layout and "exists / does not exist"
// claims are verified; projectRoot holds the PRD, the task root and .mcp.json
// and nothing an author may cite as source. Before this the authors were told
// to read "repository source under the project root", which was the working
// directory: where that is not the code repository they globbed an empty
// tree (105 live runs read the repository 0-3 times each) and wrote tasks
// claiming that files which exist "do not exist".
function groundingRules() {
  return [
    `The code repository is ${args.repositoryRoot}. It is the ONLY place to verify source paths, test names, module layout and whether a file exists or does not exist: a repository path you name must be one you observed there. Never descend into any directory named: ${excludedDirs()}. Those hold dependencies, build output and earlier runs' evidence, and are the overwhelming majority of files under that root.`,
    `${args.projectRoot} is the project root. It holds the PRD, the task root and .mcp.json, and that is all you read from it: source is read under the repository root alone, even when the two directories coincide, and a file absent from the project root is not thereby absent from the repository.`
  ].join("\n");
}

// The observation every deliverable path must carry (Issue-56). The host lint
// (`topology_lint/repository_observations.rs`) parses exactly this grammar and
// checks it against the checkout, line count included: a path with no
// observation, an inexact count, or "not observed" wording is a blocking
// finding, so the words here and the parser there must agree.
function observationRule() {
  return [
    `Every path you list under Files Expected to Change, every deliverable_contracts artifact_path and every shared_append_target_files entry is a deliverable path. For each one, read it under the repository root and write, directly after the backticked path (\`${args.repositoryRoot}/<relative path>\` or the repository-relative path), exactly one of these three observations and nothing in their place:`,
    "  \`<path>\` — exists (N lines)   for a file, where N is the last line number the Read tool shows once you have read the whole file (read again with an offset when the first read stops before the end);",
    "  \`<path>\` — exists (directory)   for a directory;",
    "  \`<path>\` — absent   for a path that is not in the repository.",
    "The host checks each observation against the repository at its recorded base commit and in the checkout, the line count exactly. A deliverable path with no such observation, an approximate or placeholder count, or words such as \"not observed\", \"could not read\" or \"outside allowed\" is a blocking finding that sends the body back to you: never defer an observation to the implementer. Never write that any repository path exists or does not exist unless you observed it under the repository root."
  ].join("\n");
}

// Directory names the host says are not worth reading, joined for a prompt.
// Sourced from args so the engine's canonical list stays the single definition.
function excludedDirs() {
  const list = Array.isArray(args.excludedDirs) ? args.excludedDirs : [];
  return list.length > 0 ? list.join(', ') : '.git, node_modules, target';
}
