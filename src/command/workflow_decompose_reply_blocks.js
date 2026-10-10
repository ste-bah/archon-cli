// Reply fences are parsed before JSON so a check script never has to be
// represented as a JSON string. Rust seed reconstruction mirrors this rule.
function replyFenceBlocks(text) {
  const source = String(text || "");
  const lines = [];
  let offset = 0;
  while (offset < source.length) {
    const end = source.indexOf("\n", offset);
    const next = end < 0 ? source.length : end + 1;
    lines.push({ start: offset, end: next, body: source.slice(offset, end < 0 ? source.length : end + 1).replace(/\r?\n$/, "") });
    offset = next;
  }
  const blocks = [];
  for (let i = 0; i < lines.length; i++) {
    const open = /^( {0,3})(`{3,})(.*)$/.exec(lines[i].body);
    if (!open || open[3].includes("`")) continue;
    const info = open[3].trim();
    let close = -1;
    for (let j = i + 1; j < lines.length; j++) {
      const closing = /^( {0,3})(`+)[ \t]*$/.exec(lines[j].body);
      if (closing && closing[2].length >= open[2].length) { close = j; break; }
    }
    if (close < 0) continue;
    let content = source.slice(lines[i].end, lines[close].start);
    content = content.replace(/(?:\r\n|\n|\r)$/, "");
    blocks.push({ info, content, start: lines[i].start, end: lines[close].end });
    i = close;
  }
  return blocks;
}

function objectText(text) {
  const trimmed = text.trim();
  if (trimmed.startsWith("[")) {
    const parsed = JSON.parse(trimmed);
    if (Array.isArray(parsed) && parsed.length > 1) throw new Error("reply contains more than one entry");
    if (Array.isArray(parsed) && parsed.length === 1) return JSON.stringify(parsed[0]);
  }
  const first = text.indexOf("{");
  const last = text.lastIndexOf("}");
  return first >= 0 && last > first ? text.slice(first, last + 1) : text.trim();
}

function jsonEqual(left, right) {
  const pending = [[left, right]];
  while (pending.length) {
    const [a, b] = pending.pop();
    if (a === b) continue;
    if (!a || !b || typeof a !== "object" || typeof b !== "object") return false;
    const aArray = Array.isArray(a), bArray = Array.isArray(b);
    if (aArray !== bArray) return false;
    const aKeys = Object.keys(a).sort(), bKeys = Object.keys(b).sort();
    if (aKeys.length !== bKeys.length || aKeys.some((key, index) => key !== bKeys[index])) return false;
    for (const key of aKeys) pending.push([a[key], b[key]]);
  }
  return true;
}

function extractJsonObject(text) {
  const raw = String(text || "");
  const blocks = replyFenceBlocks(raw);
  const jsonBlocks = blocks.filter((block) => block.info === "json");
  const outside = raw.split("");
  for (const block of blocks) for (let i = block.start; i < block.end; i++) outside[i] = " ";
  const source = outside.join("");
  let first = -1, depth = 0, quoted = false, escaped = false;
  const bare = [];
  for (let i = 0; i < source.length; i++) {
    const char = source[i];
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') quoted = false;
    } else if (char === '"') quoted = true;
    else if (char === "{") { if (depth++ === 0) first = i; }
    else if (char === "}" && depth > 0 && --depth === 0) { bare.push(source.slice(first, i + 1)); first = -1; }
  }
  const candidates = [...jsonBlocks.map((block) => objectText(block.content)), ...bare];
  if (!candidates.length) return raw.trim();
  const parsed = [];
  for (const candidate of candidates) {
    try { parsed.push(JSON.parse(candidate)); }
    catch (error) {
      if (candidates.length === 1) return candidates[0];
      const refusal = new Error(`reply contains more than one JSON object and one is not valid JSON: ${error.message}`);
      refusal.multipleJsonInvalid = true;
      throw refusal;
    }
  }
  if (parsed.length > 1 && parsed.some((value) => !jsonEqual(value, parsed[0]))) {
    throw new Error("reply contains more than one entry");
  }
  return candidates[0];
}

function resolveCommandBlock(entry, text) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) return null;
  const check = entry.check;
  const id = String(entry.id || "");
  // Only the exact info string `check` denotes the command block; `check <something>` is a different fence label.
  const blocks = replyFenceBlocks(text).filter((block) => block.info === "check");
  if (blocks.length > 1) return "acceptance reply contains more than one check block";
  const hasBlock = blocks.length === 1;
  const commandBlock = check && typeof check === "object" ? check.command_block : undefined;
  if (commandBlock === true && check && Object.prototype.hasOwnProperty.call(check, "command")) {
    return "acceptance entry " + id + " returned both check.command and check.command_block";
  }
  if (commandBlock === true && !hasBlock) return "acceptance entry " + id + " check.command_block is true but no check block is present";
  if (hasBlock && commandBlock !== true) {
    return "check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block";
  }
  if (commandBlock !== undefined && commandBlock !== true && commandBlock !== false) return "acceptance entry " + id + " check.command_block must be boolean true";
  if (!hasBlock) return null;
  if (!blocks[0].content.trim()) return "acceptance entry " + id + " check block is empty";
  check.command = blocks[0].content;
  delete check.command_block;
  return null;
}
