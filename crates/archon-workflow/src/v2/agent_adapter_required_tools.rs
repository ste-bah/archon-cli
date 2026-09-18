// The required-tool proof: which declared tools a write result must show it
// exercised, and what counts as showing it.
//
// Split from `agent_adapter_a.rs` for the 500-line ceiling. Spliced in with
// `include!` like its siblings, so it shares that file's imports and module
// scope; the callers are `validate_request_specific_result` and the tests.

/// A declared required tool: the name exactly as the task wrote it, and the
/// bare name a command would carry (`mcp__srv__quote_get` -> `quote_get`).
struct RequiredTool {
    declared: String,
    key: String,
}

/// True when the stage input declares at least one required tool the proof
/// polices — a capability such as an MCP tool, a provider CLI or a runner.
/// The write branch input embeds the task's declared required_tools (stamped
/// from the authoritative task universe), so this recognizes tasks whose
/// completion demands exercising specific tools this run.
///
/// A task whose only required tools are the host's built-in agent tools or
/// ubiquitous shell utilities is, for the no-op guard, a task with none: those
/// are how an agent inspects the tree, and inspecting the tree is precisely
/// what a typed no-op with task_coverage evidence claims to have done. Refusing
/// the no-op would demand the agent exercise a tool the guard cannot see
/// anyway (Issue-47).
fn request_declares_required_tools(input: &serde_json::Value) -> bool {
    !policed_required_tools(input).is_empty()
}

/// Every declared required tool the proof polices, deduplicated by bare name.
fn policed_required_tools(input: &serde_json::Value) -> Vec<RequiredTool> {
    let mut required = Vec::new();
    collect_required_tool_names(input, &mut required);
    required.retain(|tool| !is_unpoliced_tool(&tool.declared));
    required
}

/// EVERY declared required tool with no matching invocation in this result's
/// recorded commands — empty when all were exercised (or none were declared).
/// Each is reported by the name the task declared, so the re-asked agent is
/// told the exact identifier it was given. Reads only `required_tools` and
/// `commands_run`, with no knowledge of any specific tool, domain, or PRD, so
/// the same guard holds for every workflow engine.
///
/// A tool is exercised when a captured command names it: one of the command's
/// tokens is the bare tool name, or reduces to it through an MCP qualifier
/// (`mcp__srv__quote_get`, `mcp_action:quote_get`). Matching is by token, not
/// substring — a required `read` is not proven by `cargo test readiness`, and
/// a required `sync` is not proven by `rsync` (Issue-47).
///
/// A `skipped` entry, or one whose `output_summary` is blank or is the filler
/// the normaliser synthesises when the agent omitted it, is not proof of an
/// invocation and does not count (Issue-39). Before this, a self-reported
/// `{"command":"<tool>","status":"skipped","output_summary":"n/a"}` was enough
/// to keep an accepted verdict standing, which made the proof Issue-28 asked
/// for entirely self-reported (Issue-39, item 2).
///
/// Reports all of them, not just the first. Naming one at a time turns a single
/// contract violation into a chain of rejections that each cost an attempt:
/// observed on a review remediation where attempt 2 was rejected for one tool,
/// attempt 3 called it and was rejected for the next, and the task ran out of
/// attempts at 3 having needed 4. The agent can only fix what the rejection
/// told it about, so the rejection has to tell it everything.
fn unexercised_required_tools(input: &serde_json::Value, result: &WorkflowV2Result) -> Vec<String> {
    let required = policed_required_tools(input);
    if required.is_empty() {
        return Vec::new();
    }
    let commands: Vec<&str> = result
        .commands_run
        .iter()
        .filter(|command| command_is_a_captured_attempt(command))
        .map(|command| command.command.as_str())
        .collect();
    required
        .into_iter()
        .filter(|tool| {
            !commands
                .iter()
                .any(|command| command_names_tool(command, &tool.key))
        })
        .map(|tool| tool.declared)
        .collect()
}

/// Whether a command string names `key` (a bare, lowercased tool name) as one
/// of its tokens, directly or under an MCP qualifier.
///
/// Mirrors the Focused-Tests tokeniser in the binary's topology lint
/// (`tool_obligations.rs`), which is not reachable from this crate: tokens are
/// runs of alphanumerics, `_`, `:` and `-`, with trailing `:-_.` punctuation
/// trimmed. Keeping `-` inside a token is what stops `read-only` proving
/// `read`; keeping `:` is what lets `mcp_action:quote_get` reduce to
/// `quote_get` through `raw_tool_name`.
fn command_names_tool(command: &str, key: &str) -> bool {
    command
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == '-'))
        .map(|token| token.trim_end_matches([':', '-', '_', '.']))
        .filter(|token| !token.is_empty())
        .any(|token| {
            token.eq_ignore_ascii_case(key) || raw_tool_name(token).eq_ignore_ascii_case(key)
        })
}

/// A `commands_run` entry is proof a tool was exercised only if it actually ran
/// (any status but `skipped`) and captured what came back: a non-blank
/// `output_summary` the agent wrote itself, not the placeholder the normaliser
/// fills in when it was omitted (Issue-39).
fn command_is_a_captured_attempt(command: &WorkflowV2CommandRecord) -> bool {
    if command.status == WorkflowV2CommandStatus::Skipped {
        return false;
    }
    let summary = command.output_summary.trim();
    !summary.is_empty()
        && !summary
            .starts_with(crate::v2::agent_output_normalize::SYNTHESIZED_OUTPUT_SUMMARY_PREFIX)
}

/// A declared tool the proof does not police: a ubiquitous shell utility or
/// one of the host's own built-in agent tools, declared bare.
///
/// Judged on the name as DECLARED, never on the reduced key: an MCP server is
/// free to export a tool called `read` or `grep`, and `mcp__srv__read` is a
/// capability the guard must still demand proof of.
fn is_unpoliced_tool(declared: &str) -> bool {
    let name = declared.trim();
    if raw_tool_name(name) != name {
        return false;
    }
    is_generic_shell_utility(name) || is_builtin_agent_tool(name)
}

/// Ubiquitous shell utilities every agent already has, which this guard must
/// not police.
///
/// The guard exists to stop an agent asserting a *capability* was unavailable
/// without attempting it — a live MCP action, a provider call, a build or test
/// runner. Those can be silently skipped and their absence hidden in prose, so
/// proof of invocation is worth demanding.
///
/// A text-processing or file-listing binary is not a capability, it is a means.
/// How an agent inspects a tree is its own business, and an agent that answers
/// the same question with its own search tooling has done the work. Policing
/// these turned a satisfied task into a rejection for not shelling out to
/// `find`, which cost a run: the declaration was true, the work was done, and
/// the only thing missing was the literal binary in a command string.
///
/// Names only, no PRD or domain knowledge, so this holds for every workflow.
fn is_generic_shell_utility(tool: &str) -> bool {
    // `git` belongs here for the same reason as `grep`: agents never perform
    // git operations in this workflow — the write coordinator owns worktrees,
    // patches and commits — so a task declaring it wants the checkout INSPECTED,
    // and an agent that learned the same fact another way has done the work.
    // Leaving it out rejected a documentation audit that had already written
    // its deliverable, purely for not shelling out to the binary.
    const GENERIC: &[&str] = &[
        "awk", "basename", "bash", "cat", "cd", "cut", "diff", "dirname", "echo", "find", "git",
        "grep", "head", "ls", "mkdir", "printf", "pwd", "rg", "sed", "sh", "sort", "tail", "tee",
        "tr", "uniq", "wc", "xargs", "zsh",
    ];
    let name = tool.trim().to_ascii_lowercase();
    GENERIC.contains(&name.as_str())
}

/// The host's own built-in agent tools, which this guard must not police, for
/// the same reason as [`is_generic_shell_utility`]: a file, search, shell or
/// web primitive the agent is born with is not a capability that can be
/// quietly asserted unavailable, and `files_changed` plus artifact evidence
/// already prove the work those tools did.
///
/// There is also no evidence route for them. Built-in tool uses are host
/// observations, not commands, and the subagent outcome this adapter validates
/// carries no tool-use list — so a required `Read` could NEVER appear in
/// `commands_run` and every task declaring one was rejected on completion
/// (Issue-47: both branches of a wave finished and were refused for
/// "edit, glob, read, write").
///
/// The names are the tool names registered in `archon-tools` (`Tool::name`),
/// spelled as the host spells them, plus the aliases other agent hosts use for
/// the same primitives (`MultiEdit`, `LS`, `Task`) so a task authored in that
/// vocabulary is not rejected for it. Kept here rather than read from the
/// registry because `archon-tools` is not a dependency of this crate and
/// pulling it in for a list of names would drag the whole tool runtime along.
/// Memory tools, MCP resource tools and every `mcp__*` tool stay policed.
const BUILTIN_AGENT_TOOLS: &[&str] = &[
    "Agent",
    "ApplyPatch",
    "Bash",
    "Edit",
    "Glob",
    "Grep",
    "LS",
    "MultiEdit",
    "NotebookEdit",
    "PowerShell",
    "Read",
    "Task",
    "TodoWrite",
    "ToolSearch",
    "WebFetch",
    "WebSearch",
    "Write",
];

fn is_builtin_agent_tool(tool: &str) -> bool {
    let name = tool.trim();
    BUILTIN_AGENT_TOOLS
        .iter()
        .any(|builtin| builtin.eq_ignore_ascii_case(name))
}

/// Collect every declared required tool anywhere in the stage input, keeping
/// the declared spelling for reporting and reducing it to the bare, lowercased
/// name for matching (any `mcp__server__` or `mcp*:` qualifier stripped) so a
/// command referencing either the qualified or the raw name matches. One entry
/// per bare name: the first spelling declared is the one reported.
fn collect_required_tool_names(input: &serde_json::Value, output: &mut Vec<RequiredTool>) {
    match input {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "required_tools" | "requiredTools") {
                    if let Some(items) = value.as_array() {
                        for name in items.iter().filter_map(serde_json::Value::as_str) {
                            let bare = raw_tool_name(name).to_ascii_lowercase();
                            if !bare.is_empty() && !output.iter().any(|tool| tool.key == bare) {
                                output.push(RequiredTool {
                                    declared: name.trim().to_string(),
                                    key: bare,
                                });
                            }
                        }
                    }
                } else {
                    collect_required_tool_names(value, output);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_required_tool_names(value, output);
            }
        }
        _ => {}
    }
}
