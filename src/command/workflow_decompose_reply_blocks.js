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
    const check = /^check (\S+)$/.exec(info);
    blocks.push({ info, content, name: check && check[1], start: lines[i].start, end: lines[close].end });
  }
  return blocks;
}

function extractJsonObject(text) {
  const raw = String(text || "");
  const blocks = replyFenceBlocks(raw);
  const json = blocks.find((block) => block.info === "json");
  let body;
  if (json) body = json.content;
  else {
    const excluded = blocks.filter((block) => block.name).map((block) => [block.start, block.end]);
    const outside = raw.split("");
    for (const [start, end] of excluded) for (let i = start; i < end; i++) outside[i] = " ";
    const source = outside.join("");
    let first = -1, depth = 0, quoted = false, escaped = false;
    for (let i = 0; i < source.length; i++) {
      const char = source[i];
      if (quoted) {
        if (escaped) escaped = false;
        else if (char === "\\") escaped = true;
        else if (char === '"') quoted = false;
      } else if (char === '"') quoted = true;
      else if (char === "{") { if (depth++ === 0) first = i; }
      else if (char === "}" && depth > 0 && --depth === 0) return source.slice(first, i + 1);
    }
    return source.slice(Math.max(first, 0)).trim();
  }
  const first = body.indexOf("{");
  const last = body.lastIndexOf("}");
  return first >= 0 && last > first ? body.slice(first, last + 1) : body.trim();
}

function resolveCommandBlock(entry, text) {
  // Valid JSON can be a scalar or array. Let the caller's missing-entry path
  // refuse those values instead of reading entry.id here.
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) return null;
  const check = entry && entry.check;
  const id = String(entry.id || "");
  const named = replyFenceBlocks(text).filter((block) => block.name);
  if (!check || typeof check !== "object") return named.length ? `acceptance reply contains unreferenced check block ${named[0].name}` : null;
  if (!Object.prototype.hasOwnProperty.call(check, "command_block")) {
    return named.length ? `acceptance reply contains unreferenced check block ${named[0].name}` : null;
  }
  const name = check.command_block;
  if (Object.prototype.hasOwnProperty.call(check, "command")) return `acceptance entry ${id} returned both check.command and check.command_block`;
  if (typeof name !== "string" || !name) return `acceptance entry ${id} check.command_block must name a check block`;
  const counts = named.filter((block) => block.name === name).length;
  if (counts === 0) return `acceptance entry ${id} command_block ${name} has no matching check block`;
  if (counts > 1) return `acceptance reply contains duplicate check block name ${name}`;
  for (const block of named) if (block.name !== name) return `acceptance reply contains unreferenced check block ${block.name}`;
  const block = named.find((item) => item.name === name);
  if (!block.content.trim()) return `acceptance entry ${id} check block ${name} is empty`;
  check.command = block.content;
  delete check.command_block;
  return null;
}
