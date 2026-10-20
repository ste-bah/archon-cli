// Reply fences are parsed before JSON so a check script never has to be
// represented as a JSON string. Rust seed reconstruction mirrors this rule.
function completeJsonObjectEnd(text) {
  const start = text.search(/\S/);
  if (start < 0 || text[start] !== "{") return -1;
  const stack = [];
  let quoted = false, escaped = false;
  for (let i = start; i < text.length; i++) {
    const char = text[i];
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') quoted = false;
      continue;
    }
    if (char === '"') quoted = true;
    else if (char === "{" || char === "[") stack.push(char);
    else if (char === "}" || char === "]") {
      const expected = char === "}" ? "{" : "[";
      if (stack.pop() !== expected) return -1;
      if (!stack.length) {
        try { JSON.parse(text.slice(start, i + 1)); return i + 1; }
        catch (_) { return -1; }
      }
    }
  }
  return -1;
}

function stripReplyMarkers(text, markers) {
  const source = String(text || "");
  const configured = (Array.isArray(markers) ? markers : []).filter((marker) => typeof marker === "string" && marker.length);
  if (!configured.length) return source;
  const lines = source.match(/[^\r\n]*(?:\r\n|\r|\n|$)/g).filter(Boolean);
  let inFence = false, fenceLength = 0;
  return lines.map((line) => {
    if (inFence) {
      const body = line.replace(/[\r\n]+$/, "");
      const close = /^( {0,3})(`+)[ \t]*$/.exec(body);
      if (close && close[2].length >= fenceLength) inFence = false;
      return line;
    }
    const ending = /(?:\r\n|\r|\n)$/.exec(line)?.[0] || "";
    let body = ending ? line.slice(0, -ending.length) : line;
    // Only a marker directly between a completed JSON object and an opener
    // may split a line; every other fence opener must already start its line.
    const ticks = /`{3,}/g;
    let fenceMatch;
    while ((fenceMatch = ticks.exec(body))) {
      const prefix = body.slice(0, fenceMatch.index);
      const boundary = completeJsonObjectEnd(prefix);
      let rest = boundary >= 0 ? prefix.slice(boundary) : "", removed = false;
      while (rest) {
        const marker = configured.find((candidate) => rest.startsWith(candidate));
        if (!marker) break;
        rest = rest.slice(marker.length);
        removed = true;
      }
      const allowed = boundary >= 0 && removed && !rest.trim();
      if (allowed) { body = body.slice(0, fenceMatch.index) + "\n" + body.slice(fenceMatch.index); break; }
    }
    for (const marker of configured) body = body.split(marker).join("");
    const openLines = body.split(/\r\n|\r|\n/);
    const open = /^( {0,3})(`{3,})(.*)$/.exec(openLines[openLines.length - 1]);
    if (open && !open[3].includes("`")) inFence = true, fenceLength = open[2].length;
    return body + ending;
  }).join("");
}

function replyFenceBlocks(text, markers) {
  const source = stripReplyMarkers(text, markers);
  const lines = [];
  let offset = 0;
  while (offset < source.length) {
    const lf = source.indexOf("\n", offset);
    const cr = source.indexOf("\r", offset);
    const end = lf < 0 ? cr : cr < 0 ? lf : Math.min(lf, cr);
    if (end < 0) {
      lines.push({ start: offset, end: source.length, body: source.slice(offset) });
      break;
    }
    let next = end + 1;
    if (source[end] === "\r" && source[next] === "\n") next++;
    lines.push({ start: offset, end: next, body: source.slice(offset, end) });
    offset = next;
  }
  const blocks = [];
  for (let i = 0; i < lines.length; i++) {
    const marker = /`{3,}/.exec(lines[i].body);
    if (marker && /\S/.test(lines[i].body.slice(0, marker.index))) {
      const before = lines[i].body.slice(0, marker.index).trim();
      const quote = Array.from(before).slice(0, 80).join("") + (Array.from(before).length > 80 ? "…" : "");
      throw new Error(`fence marker on line ${i + 1} follows text; a fence must start its own line; text before the marker: ${JSON.stringify(quote)}`);
    }
    const open = /^( {0,3})(`{3,})(.*)$/.exec(lines[i].body);
    if (!open || open[3].includes("`")) continue;
    const info = open[3].trim();
    let close = -1;
    for (let j = i + 1; j < lines.length; j++) {
      const closing = /^( {0,3})(`+)[ \t]*$/.exec(lines[j].body);
      if (closing && closing[2].length >= open[2].length) { close = j; break; }
    }
    if (close < 0) {
      const midline = lines.findIndex((line) => /\S.*`{3,}/.test(line.body));
      if (midline >= 0) {
        const marker = /`{3,}/.exec(lines[midline].body);
        const before = lines[midline].body.slice(0, marker.index).trim();
        const quote = Array.from(before).slice(0, 80).join("") + (Array.from(before).length > 80 ? "…" : "");
        throw new Error(`fence marker on line ${midline + 1} follows text; a fence must start its own line; text before the marker: ${JSON.stringify(quote)}`);
      }
      for (let j = lines.length - 1; j >= 0; j--) {
        if (!lines[j].body.trim()) continue;
        if (/^ {0,3}`{3,}[ \t]*$/.test(lines[j].body)) {
          throw new Error(`closing fence on line ${j + 1} has no opener; every closing fence must close an open block`);
        }
        break;
      }
      throw new Error(`the \`\`\`${info} block opened at line ${i + 1} is never closed (the reply may have been cut off); close every block with a line that is exactly \`\`\``);
    }
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

function extractJsonObject(text, markers) {
  const raw = stripReplyMarkers(text, markers);
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

function foundFenceLabels(blocks) {
  const labels = blocks.map((block) => {
    const info = block.info || "";
    const label = Array.from(info);
    return label.length > 40 ? label.slice(0, 40).join("") + "…" : info || "<empty>";
  });
  const shown = labels.slice(0, 4).map((label) => "'" + label + "'");
  const omitted = labels.length - shown.length;
  if (omitted > 0) return shown.join(", ") + " (and " + omitted + " more)";
  return shown.length ? shown.join(", ") : "none";
}

function resolveCommandBlock(entry, text, markers) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) return null;
  const check = entry.check;
  const id = String(entry.id || "");
  // Only the exact info string `check` denotes the command block; `check <something>` is a different fence label.
  const normalizedText = stripReplyMarkers(text, markers);
  const blocks = replyFenceBlocks(normalizedText).filter((block) => block.info === "check");
  if (blocks.length > 1) return "acceptance reply contains more than one check block";
  const hasBlock = blocks.length === 1;
  const commandBlock = check && typeof check === "object" ? check.command_block : undefined;
  const inferredCommandBlock = hasBlock && check && typeof check === "object" && check.kind === "command" &&
    !Object.prototype.hasOwnProperty.call(check, "command") && !Object.prototype.hasOwnProperty.call(check, "command_block");
  if ((commandBlock === true || hasBlock) && (!check || typeof check !== "object" || check.kind !== "command")) {
    return "acceptance entry " + id + ": check.command_block is only for kind command; a floor check puts its verifier in contract.typed_verifier_command";
  }
  if (commandBlock === true && check && Object.prototype.hasOwnProperty.call(check, "command")) {
    return "acceptance entry " + id + " returned both check.command and check.command_block — remove check.command from the JSON when you use the check block";
  }
  if (hasBlock && commandBlock !== true && check && Object.prototype.hasOwnProperty.call(check, "command")) {
    return "acceptance entry " + id + " mixes inline check.command and a check fence; for a multi-line script, use block form with check.command_block true, no inline check.command, and the script in the check fence";
  }
  if (commandBlock === true && !hasBlock) return "acceptance entry " + id + " check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: " + foundFenceLabels(replyFenceBlocks(normalizedText));
  if (hasBlock && commandBlock !== true && !inferredCommandBlock) {
    return "check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block";
  }
  if (commandBlock !== undefined && commandBlock !== true && commandBlock !== false) return "acceptance entry " + id + " check.command_block must be boolean true";
  if (!hasBlock) return null;
  if (!blocks[0].content.trim()) return "acceptance entry " + id + " check block is empty";
  check.command = blocks[0].content.replace(/\r\n|\r/g, "\n");
  delete check.command_block;
  return null;
}
