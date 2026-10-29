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

function stripReplyMarkersWithMap(text, markers) {
  const source = String(text || "");
  const configured = (Array.isArray(markers) ? markers : []).filter((marker) => typeof marker === "string" && marker.length);
  if (!configured.length) return {text:source, insertedNewlineLines:[]};
  const lines = source.match(/[^\r\n]*(?:\r\n|\r|\n|$)/g).filter(Boolean);
  const insertedNewlineLines = [];
  let inFence = false, fenceLength = 0;
  const stripped = lines.map((line, lineIndex) => {
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
      if (allowed) {
        insertedNewlineLines.push(lineIndex + 2);
        body = body.slice(0, fenceMatch.index) + "\n" + body.slice(fenceMatch.index);
        break;
      }
    }
    for (const marker of configured) body = body.split(marker).join("");
    const openLines = body.split(/\r\n|\r|\n/);
    const open = /^( {0,3})(`{3,})(.*)$/.exec(openLines[openLines.length - 1]);
    if (open && !open[3].includes("`")) inFence = true, fenceLength = open[2].length;
    return body + ending;
  }).join("");
  return {text:stripped, insertedNewlineLines};
}

function stripReplyMarkers(text, markers) {
  return stripReplyMarkersWithMap(text, markers).text;
}

function replyFenceBlocks(text, markers) {
  const {text:source, insertedNewlineLines} = stripReplyMarkersWithMap(text, markers);
  const receivedLine = (lineNumber) => lineNumber - insertedNewlineLines.filter((insertedLine) => insertedLine <= lineNumber).length;
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
  const closedJsonInfoLines = [];
  for (let i = 0; i < lines.length; i++) {
    const marker = /`{3,}/.exec(lines[i].body);
    if (marker && /\S/.test(lines[i].body.slice(0, marker.index))) {
      const before = lines[i].body.slice(0, marker.index).trim();
      const quote = Array.from(before).slice(0, 80).join("") + (Array.from(before).length > 80 ? "…" : "");
      throw new Error(`fence marker on line ${receivedLine(i + 1)} follows text; a fence must start its own line; text before the marker: ${JSON.stringify(quote)}`);
    }
    const open = /^( {0,3})(`{3,})(.*)$/.exec(lines[i].body);
    if (!open || open[3].includes("`")) continue;
    const info = open[3].trim();
    let close = -1;
    for (let j = i + 1; j < lines.length; j++) {
      const closing = /^( {0,3})(`+)[ \t]*$/.exec(lines[j].body);
      if (closing && closing[2].length >= open[2].length) { close = j; break; }
    }
    const infoClosers = [];
    const contentEnd = close < 0 ? lines.length : close;
    for (let j = i + 1; j < contentEnd; j++) {
      const nested = /^( {0,3})(`{3,})(.*)$/.exec(lines[j].body);
      if (nested && nested[2].length >= open[2].length) {
        const info = nested[3].trim();
        if (info) infoClosers.push({line:j + 1, info});
      }
    }
    // A genuinely unclosed block is structural. For a closed JSON block, defer
    // the info-line finding until we know whether JSON/check resolution failed.
    if (close < 0 && infoClosers.length) {
      const first = infoClosers[0];
      const duplicate = infoClosers.filter((candidate) => candidate.info === "check")[1];
      const displayedInfo = first.info === "check" ? "check" : "<info>";
      const duplicateText = duplicate
        ? `; line ${receivedLine(duplicate.line)} is a duplicate check fence opener` : "";
      throw new Error(`the fence opened on line ${receivedLine(i + 1)} is not closed; line ${receivedLine(first.line)} \"\`\`\`${displayedInfo}\" has an info string and cannot close it${duplicateText}; put a bare \`\`\` on its own line before the next fence; keep exactly one non-empty check fence for block form`);
    }
    if (infoClosers.length && info === "json") {
      closedJsonInfoLines.push({ blockContent: source.slice(lines[i].end, lines[close].start).replace(/(?:\r\n|\n|\r)$/, ""),
        openedLine: i + 1, line: infoClosers[0].line, info: infoClosers[0].info,
        duplicateCheckLine: infoClosers.filter((candidate) => candidate.info === "check")[1]?.line });
    }
    if (close < 0) {
      const midline = lines.findIndex((line) => /\S.*`{3,}/.test(line.body));
      if (midline >= 0) {
        const marker = /`{3,}/.exec(lines[midline].body);
        const before = lines[midline].body.slice(0, marker.index).trim();
        const quote = Array.from(before).slice(0, 80).join("") + (Array.from(before).length > 80 ? "…" : "");
        throw new Error(`fence marker on line ${receivedLine(midline + 1)} follows text; a fence must start its own line; text before the marker: ${JSON.stringify(quote)}`);
      }
      for (let j = lines.length - 1; j >= 0; j--) {
        if (!lines[j].body.trim()) continue;
        if (/^ {0,3}`{3,}[ \t]*$/.test(lines[j].body)) {
          throw new Error(`closing fence on line ${receivedLine(j + 1)} has no opener; every closing fence must close an open block`);
        }
        break;
      }
      throw new Error(`the \`\`\`${info} block opened at line ${receivedLine(i + 1)} is never closed (the reply may have been cut off); close every block with a line that is exactly \`\`\``);
    }
    let content = source.slice(lines[i].end, lines[close].start);
    content = content.replace(/(?:\r\n|\n|\r)$/, "");
    blocks.push({ info, content, start: lines[i].start, end: lines[close].end });
    i = close;
  }
  if (!blocks.some((block) => block.info === "check")) {
    const defect = closedJsonInfoLines.find(({blockContent}) => {
      let entry;
      try { entry = JSON.parse(blockContent); } catch (_) { return true; }
      return entry && entry.check && entry.check.command_block === true;
    });
    if (defect) {
      const displayedInfo = defect.info === "check" ? "check" : "<info>";
      const duplicateText = defect.duplicateCheckLine
        ? `; line ${receivedLine(defect.duplicateCheckLine)} is a duplicate check fence opener` : "";
      throw new Error(`the fence opened on line ${receivedLine(defect.openedLine)} is not closed; line ${receivedLine(defect.line)} \"\`\`\`${displayedInfo}\" has an info string and cannot close it${duplicateText}; put a bare \`\`\` on its own line before the next fence; keep exactly one non-empty check fence for block form`);
    }
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
  const blocks = replyFenceBlocks(text, markers);
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
  const blocks = replyFenceBlocks(text, markers).filter((block) => block.info === "check");
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
    return "acceptance entry " + id + " mixes an inline command with a check fence and must use one form";
  }
  if (commandBlock === true && !hasBlock) return "acceptance entry " + id + " check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: " + foundFenceLabels(replyFenceBlocks(text, markers));
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

function parseErrorClass(error) {
  // Match a finite set of parser kinds. The message itself is diagnostic
  // text and must never become a progress key.
  const message = String(error && error.message || error || "").toLowerCase();
  let kind = "other";
  if (/unterminated string/.test(message)) kind = "unterminated-string";
  else if (/unexpected end|end of json|unexpected eof/.test(message)) kind = "unexpected-end";
  else if (/expected property name|property name enclosed/.test(message)) kind = "property-name";
  else if (/expected .*after property|expected colon/.test(message)) kind = "property-separator";
  else if (/unexpected token|unexpected character|invalid \\escape/.test(message)) kind = "unexpected-token";
  // Retain numeric source coordinates so repair tracking can compare how far
  // the reply parses. Do not let provider-controlled diagnostic text enter
  // progress state.
  const lineColumn = message.match(/line\s+(\d+)\s+column\s+(\d+)/);
  const position = message.match(/(?:position|offset|char)\s+(\d+)/);
  const location = lineColumn ? `line-${lineColumn[1]}-column-${lineColumn[2]}`
    : position ? `position-${position[1]}` : "location-unknown";
  return `parse-error:${kind}:${location}`;
}

function stopReasonClass(reason) {
  const known = new Set(["max_tokens", "tool_use", "refusal", "pause_turn"]);
  if (reason === undefined || reason === null || reason === "") return "stop-missing";
  return known.has(reason) ? `stop-${reason.replace(/_/g, "-")}` : "stop-other";
}

function redactSecretShapedText(text) {
  if (typeof __archonRedactPublicText !== "function") {
    throw new Error("host public-text redactor binding __archonRedactPublicText is required");
  }
  return __archonRedactPublicText(String(text));
}

function redactFinding(finding) {
  if (typeof finding === "string") return redactSecretShapedText(finding);
  if (!finding || typeof finding !== "object") return finding;
  return typeof finding.text === "string"
    ? {...finding, text:redactSecretShapedText(finding.text)}
    : finding;
}

function redactFailure(failure) {
  if (!failure || typeof failure !== "object") return failure;
  if (Array.isArray(failure.findings)) failure.findings = failure.findings.map(redactFinding);
  if (typeof failure.summary === "string") failure.summary = redactSecretShapedText(failure.summary);
  return failure;
}

function escapeDiagnosticControls(value) {
  return Array.from(value, (character) => {
    const code = character.codePointAt(0);
    if (character === "\n") return "\\n";
    if (character === "\r") return "\\r";
    if (character === "\t") return "\\t";
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return `\\u{${code.toString(16)}}`;
    return character;
  }).join("");
}

function commandScriptNeedsBlock(source, offset) {
  if (offset === null) return false;
  const limit = Math.min(source.length, Math.max(0, offset));
  const stack = [];
  for (let index = 0; index < limit;) {
    const character = source[index];
    const parent = stack[stack.length - 1];
    if (character === '"') {
      const isKey = parent && parent.type === "object" && parent.expectKey;
      const isCommand = !isKey && stack.length === 2 && stack[0].isTop && stack[1].isCheck
        && stack[1].key === "command";
      const valueStart = index + 1;
      let cursor = valueStart, escapedNewline = false, closed = false;
      for (; cursor < source.length; cursor++) {
        if (source[cursor] === "\\") {
          const escaped = source[cursor + 1];
          if (escaped === "n" || escaped === "r") escapedNewline = true;
          cursor += escaped === "u" && /^[0-9a-fA-F]{4}$/.test(source.slice(cursor + 2, cursor + 6)) ? 5 : 1;
        } else if (source[cursor] === '"') {
          closed = true;
          break;
        }
      }
      if (isCommand && offset >= valueStart && (!closed || offset <= cursor)) {
        return hasEscapedLineBreak(source.slice(valueStart, Math.min(offset, cursor)))
          || hasEscapedLineBreak(source.slice(Math.min(offset, cursor), cursor));
      }
      if (isCommand && closed && cursor < limit && escapedNewline) return true;
      const raw = source.slice(valueStart, Math.min(cursor, limit));
      if (isKey && closed && cursor < limit) {
        try { parent.key = JSON.parse(`"${raw}"`); } catch { parent.key = ""; }
        parent.expectKey = false;
      }
      index = Math.min(closed ? cursor + 1 : source.length, limit);
      continue;
    }
    if (character === "{" || character === "[") {
      const isTop = stack.length === 0 && character === "{";
      const isCheck = character === "{" && stack.length === 1 && parent.isTop && parent.key === "check";
      stack.push({type:character === "{" ? "object" : "array", key:null, expectKey:character === "{", isTop, isCheck});
    } else if (character === "}" || character === "]") {
      stack.pop();
    } else if (character === "," && parent && parent.type === "object") {
      parent.key = null;
      parent.expectKey = true;
    }
    index++;
  }
  return false;
}

function hasEscapedLineBreak(value) {
  for (let index = 0; index < value.length; index++) {
    if (value[index] !== "\\" || index + 1 >= value.length) continue;
    const escaped = value[index + 1];
    if (escaped === "n" || escaped === "r") return true;
    index += escaped === "u" && /^[0-9a-fA-F]{4}$/.test(value.slice(index + 2, index + 6)) ? 5 : 1;
  }
  return false;
}

// Keep malformed-reply refusals useful to the author without echoing the
// whole reply. JSON.parse engines report positions in different forms; retain
// their message and derive a nearby excerpt when a location is available.
// The joined excerpt is redacted before adding its fault marker.
function acceptanceParseRefusal(id, source, error) {
  const message = redactSecretShapedText(String(error && error.message || error || "invalid JSON")
    .replace(/[\r\n\t]+/g, " ")).slice(0, 240);
  let offset = null;
  let location = "";
  const lineColumn = /line\s+(\d+)\s+column\s+(\d+)/i.exec(message);
  const byteOffset = /byte(?:\s+offset)?\s+(\d+)/i.exec(message);
  const position = /(?:position|at)\s+(\d+)/i.exec(message);
  if (lineColumn) {
    const lines = source.split("\n");
    const line = Number(lineColumn[1]);
    const column = Number(lineColumn[2]);
    if (line > 0 && line <= lines.length && column > 0) {
      offset = lines.slice(0, line - 1).reduce((sum, value) => sum + value.length + 1, 0)
        + Math.min(column - 1, lines[line - 1].length);
      location = ` at line ${line} column ${column}`;
    }
  } else if (byteOffset) {
    const bytes = Number(byteOffset[1]);
    let seen = 0;
    offset = 0;
    while (offset < source.length && seen < bytes) {
      const code = source.codePointAt(offset);
      seen += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
      offset += code > 0xffff ? 2 : 1;
    }
    location = ` at byte offset ${bytes}`;
  } else if (position) {
    offset = Math.min(Number(position[1]), source.length);
    location = ` at offset ${offset}`;
  }
  const blockDirection = commandScriptNeedsBlock(source, offset)
    ? ' the check is a multi-line script; return it in block form instead: set "command_block": true in the entry and put the script unescaped in exactly one fence labelled check after the JSON entry; then no escaping is needed.'
    : "";
  let excerpt = "";
  if (offset !== null) {
    // An invalid escape can split a credential field name before the shared
    // redactor sees it. Treat that malformed spelling as the joined field so
    // its value is still removed in the diagnostic. Redact the source first
    // so a credential cut by either window edge cannot leave a partial value.
    const redactionSource = source.replace(/api-\\qkey/gi, "api-key");
    const redactedSource = redactSecretShapedText(redactionSource);
    const redactPrefix = (end) => redactSecretShapedText(redactionSource.slice(0, end)).length;
    const lineStart = source.lastIndexOf("\n", Math.max(0, offset - 1)) + 1;
    const newline = source.indexOf("\n", offset);
    const lineEnd = newline < 0 ? source.length : newline;
    const mappedStart = redactPrefix(lineStart);
    const mappedOffset = redactPrefix(offset);
    const mappedEnd = redactPrefix(lineEnd);
    const line = Array.from(redactedSource.slice(mappedStart, mappedEnd));
    const split = Math.max(0, mappedOffset - mappedStart);
    const before = line.slice(Math.max(0, split - 40), split).join("");
    const after = line.slice(split, split + 40).join("");
    const joined = escapeDiagnosticControls(before + after);
    const sourceBefore = Array.from(source.slice(lineStart, offset)).slice(-40).join("");
    const sourceAfter = Array.from(source.slice(offset, lineEnd)).slice(0, 40).join("");
    const sourceJoined = escapeDiagnosticControls(sourceBefore + sourceAfter);
    const normalizedJoined = sourceJoined.replace(/api-\\qkey/gi, "api-key");
    const redactedJoined = redactSecretShapedText(normalizedJoined);
    const redactionChangedExcerpt = redactedJoined !== normalizedJoined || joined !== sourceJoined;
    if (redactionChangedExcerpt) {
      excerpt = joined;
    } else {
      const escapedBefore = Array.from(escapeDiagnosticControls(before));
      const escapedJoined = Array.from(joined);
      excerpt = `${escapedBefore.join("")}<HERE>${escapedJoined.slice(escapedBefore.length).join("")}`;
    }
  }
  const excerptText = excerpt ? `; excerpt: ${JSON.stringify(excerpt)}` : "";
  const escapeRule = /invalid \\escape|bad escaped character/i.test(String(error && error.message || error || ""))
    ? "; a literal backslash inside a JSON string must be written as `\\\\`" : "";
  return `acceptance entry ${id} returned no complete entry: JSON parse error${location}: ${message}${excerptText}${escapeRule}.${blockDirection} Choose exactly one command form: for scripts, use the block form with kind and cwd, command_block true and no command field in the JSON check, followed by exactly one non-empty block whose fence info string is exactly 'check'; inline form puts one short line without quotes or backslashes in check.command with kind and cwd, and has no command_block or fence. Never write a script inside a JSON string.`;
}

function fencedEntryLinePrefix(reply, markers) {
  const block = replyFenceBlocks(stripReplyMarkers(reply, markers)).find((candidate) => candidate.info === "json");
  if (!block) return 0;
  const openerLine = String(reply).slice(0, block.start).split(/\r\n|\r|\n/).length;
  return openerLine;
}
