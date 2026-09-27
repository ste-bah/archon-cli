//! Obs-119: a declared check that runs against live application state.
//!
//! A task declared MCP compile and error-listing tools of an external
//! editor. Those tools act on whatever script the editor holds, not on the
//! repository's source: a verifier reported another script's errors as the
//! task's, and a compile tool pressed the editor's Save on that script.
//!
//! This section tells a call whose declared tools include such a tool to
//! load the task's declared source into the application first (with a
//! declared loader, which it names when one is declared), to say so when it
//! cannot rather than attribute the result to the task, and never to call a
//! save/publish-style tool the task does not declare. It is rendered only
//! for MCP-qualified declared tools, classified by NAME ALONE because the
//! host holds no tool metadata at prompt time; the word lists below are
//! generic verbs and nouns, never tool or server names:
//!
//! - a CHECK tool's bare name has a word in [`CHECK_WORDS`] (`x_compile`,
//!   `get_errors`, `lint`); the agent, which sees each tool's input schema,
//!   treats one as live-state bound only when it examines content and takes
//!   no source input (a `health_check` examines none);
//! - a LOADER's bare name pairs a word in [`LOAD_VERBS`] with one in
//!   [`SOURCE_NOUNS`] (`set_source`, `open_file`, `load_script`);
//! - a SAVER's bare name has a word in [`SAVE_VERBS`] (`save`, `publish`).
//!
//! Undeclared MCP tools are never bound to a workflow agent (the binary's
//! `allowed_mcp_tools`), so the ban on undeclared savers restates what the
//! binding enforces; a DECLARED check tool that saves as a side effect can
//! only be named, so the section asks for the side effect to be reported.
//!
//! The adapter does NOT refuse a compile result that no loader call preceded
//! (Obs-119's evidence rule): the name rule cannot tell a live-state check
//! from one that takes the source as input, the host's tool log keeps only
//! a clipped input head (a loader's source text cannot be matched to a
//! declared artifact), verifier sessions record no host tool calls and the
//! required-tool proof does not apply to read-only results, and a task that
//! declares no loader could never satisfy the rule, since only declared MCP
//! tools are bound. Refusing on it would be a false refusal or a dead end.

use crate::tool_declarations::raw_tool_name;

/// Words naming a check, compile or diagnostics step.
pub(super) const CHECK_WORDS: &[&str] = &[
    "compile",
    "check",
    "lint",
    "errors",
    "console",
    "diagnostics",
    "validate",
    "analyze",
    "analyse",
];
/// Verbs that put content into an application.
pub(super) const LOAD_VERBS: &[&str] = &["set", "load", "open", "replace", "put", "import"];
/// Nouns naming the content a loader puts.
pub(super) const SOURCE_NOUNS: &[&str] = &["source", "script", "code", "file", "document"];
/// Verbs that persist or publish application content.
pub(super) const SAVE_VERBS: &[&str] = &[
    "save", "publish", "commit", "deploy", "delete", "remove", "push", "upload", "release",
];

/// The live-state section for `input`'s declared tools; empty when none of
/// them is an MCP check tool.
pub(crate) fn live_state_tools_prompt_section(input: &serde_json::Value) -> String {
    let mut declared = Vec::new();
    collect(input, &mut declared);
    let words = |name: &str| -> Vec<String> {
        raw_tool_name(name)
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    };
    let has = |name: &str, list: &[&str]| words(name).iter().any(|w| list.contains(&w.as_str()));
    let loader = |name: &str| has(name, LOAD_VERBS) && has(name, SOURCE_NOUNS);
    let checks: Vec<&String> = declared
        .iter()
        .filter(|name| has(name, CHECK_WORDS) && !loader(name))
        .collect();
    if checks.is_empty() {
        return String::new();
    }
    let loaders: Vec<&String> = declared.iter().filter(|name| loader(name)).collect();
    let savers: Vec<&String> = declared
        .iter()
        .filter(|name| has(name, SAVE_VERBS))
        .collect();
    let join = |names: &[&String]| {
        names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let load = if loaders.is_empty() {
        "a tool that sets or loads the source; this task declares none, so you cannot load it"
            .to_string()
    } else {
        format!("the declared loader {}", join(&loaders))
    };
    let saved = if savers.is_empty() {
        String::new()
    } else {
        format!(
            " This task declares {}: call it only as the task says.",
            join(&savers)
        )
    };
    format!(
        "## Live-State Tools\n\
         Declared tools that may check content in an external application: {}. One that \
         examines content (code, a script, a document) but whose input schema takes no source \
         input acts on whatever that application currently holds (an open editor or \
         document), not on this repository's files. For each such tool: load this task's \
         declared source artifact into the application first, in this session and \
         immediately before the check, with {load}. If the source is not loaded, the check did \
         not examine this task's artifact: still record the call and its verbatim output in \
         commands_run, state in its output_summary that it ran against the application's \
         current content, and never count its errors or its success as this task's. Never call \
         a tool that saves, publishes, commits, deploys or deletes application content unless \
         this task declares it.{saved} If a declared check tool saves or changes the \
         application's content as a side effect, say so in its output_summary.\n\n",
        join(&checks)
    )
}

/// Every MCP-qualified name under a `required_tools`/`requiredTools` key,
/// at any depth, deduplicated in order.
fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if !matches!(key.as_str(), "required_tools" | "requiredTools") {
                    collect(value, out);
                    continue;
                }
                for name in value.as_array().into_iter().flatten() {
                    let Some(name) = name.as_str().map(str::trim) else {
                        continue;
                    };
                    if raw_tool_name(name) != name && !out.iter().any(|seen| seen == name) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        serde_json::Value::Array(values) => values.iter().for_each(|value| collect(value, out)),
        _ => {}
    }
}

#[cfg(test)]
#[path = "agent_prompt_live_tools_tests.rs"]
mod tests;
