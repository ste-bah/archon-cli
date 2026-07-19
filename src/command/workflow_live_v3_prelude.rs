// v3 dialect support: the Claude-Code-style primitive layer injected into
// workflow scripts marked by `export const meta`, and the export
// normalization shared by every dialect. Split from script_helpers to
// respect the 500-line source ceiling.

const V3_PRIMITIVES_JS: &str = r#"
// v3 script dialect (marked by `export const meta`): Claude-Code-style
// primitives layered over the host API. Call ids derive deterministically
// from labels + ordinals so unchanged prefixes replay from cache.
function __archonPrimitives(w) {
  let ordinal = 0;
  let phaseIndex = 0;
  // phase()/log() are UI/journal markers: Claude Code scripts call them
  // without await. Their checkpoint promises are collected here and flushed
  // by the runner after the workflow returns, so they can neither be dropped
  // nor trip the pending-call guard.
  globalThis.__archonMarkers = [];
  const slug = (text) =>
    String(text).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "step";
  // Early UX guard mirroring the HOST rule (authoritative copy runs in the
  // dry-run recorder): literal repo-relative paths — no whitespace, no
  // traversal, not absolute, no globs. Extensionless files like Makefile are
  // valid. Write work requires a non-empty list.
  const assertPathList = (list, what, requireNonEmpty) => {
    if (requireNonEmpty && (!Array.isArray(list) || list.length === 0)) {
      throw new Error(`${what} must list at least one literal repo-relative file path for write work`);
    }
    for (const entry of list || []) {
      const bad =
        typeof entry !== "string" ||
        entry.trim() === "" ||
        /\s/.test(entry) ||
        entry.startsWith("/") ||
        entry.split("/").includes("..") ||
        /[*?\[]/.test(entry);
      if (bad) {
        throw new Error(`${what} entries must be literal repo-relative file paths (no whitespace, traversal, globs, or absolute paths; got: ${JSON.stringify(entry).slice(0, 120)})`);
      }
    }
  };
  const agent = async (prompt, opts = {}) => {
    if (typeof prompt !== "string" || prompt.trim() === "") {
      throw new Error("agent(prompt, opts) requires a non-empty prompt string");
    }
    ordinal += 1;
    const id = `${slug(opts.label || "agent")}-${ordinal}`;
    if (opts.write) {
      assertPathList(opts.targetFiles, "agent() targetFiles", true);
      const item = {
        item_id: id,
        canonical_task_ids: opts.taskIds || [],
        task: prompt,
        instructions: prompt,
        target_files: opts.targetFiles || [],
        focused_verification: opts.focusedTests || [],
        artifact_requirements: opts.artifacts || [],
        work_type: "implementation",
      };
      return await w.fanout(id, [item], {
        write: "worktree",
        itemKind: "implementation",
        tier: opts.tier || "coder",
        targetFilesFromItem: true,
        maxParallelism: 1,
        task: prompt,
      });
    }
    // Per-task verifiers (verify:true or focusedTests) run through the
    // HOST's verification-wave machinery: the wave id prefix grants command
    // execution and attaches every focused-verification guard (zero-test
    // demotion, outcome normalization, host-anchored evidence). The
    // adversarial reviewer stays a plain read-only agent by design.
    if (opts.verify === true || (Array.isArray(opts.focusedTests) && opts.focusedTests.length > 0)) {
      const item = {
        item_id: `${id}-check`,
        canonical_task_ids: opts.taskIds || [],
        task: prompt,
        instructions: prompt,
        focused_verification: opts.focusedTests || [],
        artifact_requirements: opts.artifacts || [],
      };
      return await w.parallel(`verification-wave-${id}`, [item], {
        tier: opts.tier || "coder",
        itemKind: "focused_verification",
        task: prompt,
      });
    }
    return await w.agent(id, {
      tier: opts.tier || "coder",
      task: prompt,
      targetFiles: opts.targetFiles || [],
    });
  };
  // Batch form: N specs through ONE host fanout/parallel call, so the HOST
  // controls safe concurrency (worktree isolation, build-lock serialization).
  const agents = async (specs, opts = {}) => {
    if (!Array.isArray(specs) || specs.length === 0) {
      throw new Error("agents(specs, opts) requires a non-empty array of {prompt, label, ...} specs");
    }
    ordinal += 1;
    const id = `${slug(opts.label || "agents")}-${ordinal}`;
    const cargoish = specs.some((spec) =>
      [spec.prompt, ...(spec.focusedTests || [])].join(" ").includes("cargo "));
    const items = specs.map((spec, index) => {
      if (typeof spec.prompt !== "string" || spec.prompt.trim() === "") {
        throw new Error("every agents() spec requires a non-empty prompt");
      }
      if (opts.write) {
        assertPathList(spec.targetFiles, "agents() spec targetFiles", true);
      }
      return {
        item_id: `${id}-${slug(spec.label || `item-${index + 1}`)}`,
        canonical_task_ids: spec.taskIds || [],
        task: spec.prompt,
        instructions: spec.prompt,
        target_files: spec.targetFiles || [],
        focused_verification: spec.focusedTests || [],
        artifact_requirements: spec.artifacts || [],
        work_type: opts.write ? "implementation" : "verification",
      };
    });
    if (opts.write) {
      return await w.fanout(id, items, {
        write: "worktree",
        itemKind: "implementation",
        tier: opts.tier || "coder",
        targetFilesFromItem: true,
        maxParallelism: cargoish ? 1 : opts.maxParallelism,
        task: opts.task || "Execute every item in this batch.",
      });
    }
    return await w.parallel(id, items, {
      tier: opts.tier || "coder",
      maxParallelism: cargoish ? 1 : opts.maxParallelism,
      task: opts.task || "Execute every item in this batch.",
    });
  };
  const phase = (title, body) => {
    phaseIndex += 1;
    const marker = w.checkpoint(`phase-${phaseIndex}-${slug(title)}`, {
      task: `Phase: ${String(title).slice(0, 200)}`,
    });
    globalThis.__archonMarkers.push(marker);
    // Three valid styles: bare `phase('T')` (Claude Code form, no await
    // needed), `await phase('T')`, or `await phase('T', async () => {...})`
    // which runs and awaits the body and returns its result — silently
    // ignoring a body function would drop entire phases of real work.
    if (typeof body === "function") {
      return (async () => {
        await marker;
        return await body();
      })();
    }
    return marker;
  };
  const log = (message) => {
    ordinal += 1;
    const marker = w.checkpoint(`log-${ordinal}`, {
      task: `Log: ${String(message).slice(0, 400)}`,
    });
    globalThis.__archonMarkers.push(marker);
    return marker;
  };
  const pipeline = async (items, stages) => {
    if (!Array.isArray(items) || !Array.isArray(stages)) {
      throw new Error("pipeline(items, stages) requires an items array and a stages array of async functions");
    }
    const results = [];
    for (const item of items) {
      let current = item;
      for (const stage of stages) {
        current = await stage(current);
      }
      results.push(current);
    }
    return results;
  };
  return Object.freeze({ agent, agents, phase, log, pipeline, w });
}
"#;

fn normalize_workflow_export(source: &str) -> String {
    let mut normalized = source.trim().to_string();
    // v3 dialect marker: `export const meta` becomes a plain const plus the
    // global flag __archonRun uses to hand the script the primitive API.
    if let Some(offset) = workflow_meta_marker_offset(&normalized) {
        normalized.replace_range(offset..offset + "export const meta".len(), "const meta");
        // The genuine Claude Code script shape is TOP-LEVEL code after the
        // meta export — no wrapper function. Wrap everything after the meta
        // statement so top-level `await` and `return` become legal, with the
        // primitives available as globals.
        if !has_workflow_function_declaration(&normalized) {
            let body_start = statement_end_offset(&normalized, offset);
            let body = normalized.split_off(body_start);
            normalized.push_str("\nasync function workflow() {\n");
            normalized.push_str(&body);
            normalized.push_str("\n}");
        }
        normalized.insert_str(0, "globalThis.__workflowMeta = true;\n");
    }
    // QuickJS evaluates non-module source: neutralize the default export
    // wherever it appears (v3 scripts put `export const meta` first).
    for (from, to) in [
        (
            "export default async function workflow",
            "async function workflow",
        ),
        ("export default function workflow", "function workflow"),
        (
            "export default async function(",
            "async function workflow(",
        ),
        ("export default function(", "function workflow("),
    ] {
        if normalized.contains(from) {
            normalized = normalized.replacen(from, to, 1);
            break;
        }
    }
    normalized
        .replace("export default workflow;", "")
        .replace("export default workflow", "")
}

fn has_workflow_function_declaration(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        [
            "export default async function workflow",
            "export default function workflow",
            "async function workflow",
            "function workflow",
        ]
        .iter()
        .any(|declaration| line.starts_with(declaration))
    })
}

/// Offset just past the end of the statement starting at `start` — the
/// balanced close of its first `{...}` block plus an optional trailing `;`.
/// Quote- and escape-aware so braces inside meta strings don't miscount.
fn statement_end_offset(source: &str, start: usize) -> usize {
    let bytes = &source[start..];
    let Some(open) = bytes.find('{') else {
        return source.len();
    };
    let mut depth = 0usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (offset, ch) in bytes[open..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string.is_some() => escaped = true,
            '"' | '\'' | '`' => match in_string {
                Some(quote) if quote == ch => in_string = None,
                None => in_string = Some(ch),
                _ => {}
            },
            '{' if in_string.is_none() => depth += 1,
            '}' if in_string.is_none() => {
                depth -= 1;
                if depth == 0 {
                    let mut end = start + open + offset + ch.len_utf8();
                    if source[end..].starts_with(';') {
                        end += 1;
                    }
                    return end;
                }
            }
            _ => {}
        }
    }
    source.len()
}

fn workflow_meta_marker_offset(source: &str) -> Option<usize> {
    let marker = "export const meta";
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let declaration = line.trim_start();
        if let Some(rest) = declaration.strip_prefix(marker)
            && rest
                .chars()
                .next()
                .is_none_or(|ch| ch.is_ascii_whitespace() || ch == '=')
        {
            return Some(offset + line.len() - declaration.len());
        }
        offset += line.len();
    }
    None
}
