// Prompt spans and overflow handling, hoisted into the fixed script scope.
function requireFixedArgs() {
  if (!args || typeof args !== "object") throw new Error("fixed decomposition args are absent");
  for (const key of ["projectRoot", "repositoryRoot", "prdPath", "prdDigest", "taskRoot"]) {
    if (typeof args[key] !== "string" || args[key].trim() === "") {
      throw new Error(`fixed decomposition argument ${key} is missing`);
    }
  }
  if (args.gateMode !== "observe" && args.gateMode !== "enforce") {
    throw new Error("fixed decomposition requires observe or enforce gate mode");
  }
}

// The live roots a check must not name, as the validator reads them: the
// roots this prompt names, and the task root the host reads the freeze's own
// roots from (its configured scratch repository), so both steps refuse a
// check from one source.
function liveRootsText() {
  return JSON.stringify({repository: args.repositoryRoot, project: args.projectRoot, tasks: args.taskRoot});
}

// Each task owns exact optional spans; required text is never searched/replaced.
function optionalAuthorContext(name, text, blocks, start = 0) {
  if (text && blocks) blocks.push({ name, text, start });
  return text;
}

function requiredAuthorContext(name, text, blocks, start = 0) {
  if (text && blocks) blocks.push({ name, text, start, required: true });
  return text;
}

// Keep ordinary findings byte-identical; the reducer alone uses the subset.
function findingsAuthorContext(name, text, blocks, start = 0, lines = []) {
  optionalAuthorContext(name, text, blocks, start);
  if (text && blocks) blocks[blocks.length - 1].lines = lines;
  return text;
}

async function requireDispatchable(w, subject, task, blocks = [], tail = "") {
  // Preserve the last named-files-preamble-sized band too: namedFilesContext
  // omits its optional instructions when they do not fit, as on the main branch.
  if (utf8Bytes(task + tail) <= AUTHOR_PROMPT_LIMIT) return task;
  const ranked = blocks.filter(block => !block.required && block.text)
    .sort((a, b) => utf8Bytes(b.text) - utf8Bytes(a.text) || a.start - b.start);
  const render = replacements => {
    let out = task;
    for (const [block, text] of [...replacements].sort((a, b) => b[0].start - a[0].start)) {
      if (task.slice(block.start, block.start + block.text.length) !== block.text) throw new Error("invalid optional context span");
      out = out.slice(0, block.start) + text + out.slice(block.start + block.text.length);
    }
    return out;
  };
  const scaffold = render(new Map(ranked.map(block => [block, ""])));
  const bytes = utf8Bytes(scaffold + tail);
  if (bytes > AUTHOR_PROMPT_LIMIT) {
    const fixed = blocks.filter(block => block.required && task.slice(block.start, block.start + block.text.length) === block.text);
    const namedBytes = fixed.reduce((sum, block) => sum + utf8Bytes(block.text), 0);
    const parts = [...new Set([
      ...fixed.map(block => block.name),
      ...(namedBytes < utf8Bytes(scaffold) ? [fixed.length ? "other fixed scaffold text" : "prompt text"] : []),
      ...(tail ? ["reply format"] : [])
    ])].join(", ");
    await stopFixed(`Author prompt refused for ${subject}: fixed scaffold (${parts}) uses ${bytes} UTF-8 bytes; limit ${AUTHOR_PROMPT_LIMIT} bytes. This limit is not configurable.`);
  }
  const moved = [];
  const replacements = new Map();
  let reduced = task;
  for (const block of ranked) {
    // Raw candidate files are written only after this block is selected to move.
    const stub = block.raw !== undefined
      ? `Read the retained candidate as raw text at ${contextFile("txt", block.raw).path} (host-written body context for this call: read it by its exact path, even under an excluded directory).`
      : `${block.text.match(/^\s*/)[0]}${block.text.trimStart().split("\n")[0].split(":")[0]}: [moved to context file]`;
    moved.push({ name: block.name, text: block.raw !== undefined ? stub : block.text });
    replacements.set(block, stub);
    const file = contextFile("json", JSON.stringify(moved));
    const reference = `\nMoved optional blocks: ${moved.map(part => part.name).join(", ")}. Read ${file.path} (host-written context for this call: read it by its exact path).`;
    reduced = render(replacements) + reference;
    if (utf8Bytes(reduced + tail) <= AUTHOR_PROMPT_LIMIT) break;
  }
  // Authored content cannot cause refusal even when a reference is the last
  // overflow. Remove optional stubs before sacrificing the context index.
  if (utf8Bytes(reduced + tail) > AUTHOR_PROMPT_LIMIT) {
    for (const block of ranked) replacements.set(block, "");
    const file = contextFile("json", JSON.stringify(moved));
    const reference = `\nRead full authored context at ${file.path}.`;
    reduced = scaffold + (bytes + utf8Bytes(reference) <= AUTHOR_PROMPT_LIMIT ? reference : "");
  }
  // Add bounded first entries by their exact registered span.
  const rendered = render(replacements);
  const suffix = reduced.startsWith(rendered) ? reduced.slice(rendered.length) : null;
  if (suffix !== null) {
    for (const [block, stub] of replacements) {
      if (!block.lines?.length || !stub) continue;
      const room = Math.min(HISTORY_SHARE, AUTHOR_PROMPT_LIMIT - utf8Bytes(reduced + tail));
      const count = `\n${block.lines.length} findings in full context; first entries that fit:`;
      if (utf8Bytes(count) > room) continue;
      const lines = fitLines(block.lines, room - utf8Bytes(count));
      replacements.set(block, stub + count + lines.map(line => `\n- ${line}`).join(""));
      reduced = render(replacements) + suffix;
    }
  }
  return reduced;
}
