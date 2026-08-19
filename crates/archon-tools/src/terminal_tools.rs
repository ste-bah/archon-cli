//! Persistent shell sessions the model can call (#189 Phase 6).
//!
//! `Bash` is one-shot: every call is a fresh process, so the agent cannot `cd`
//! and stay there, cannot activate an environment and reuse it, cannot run
//! anything interactive, and cannot start something long and check on it later.
//! These four tools are the missing half — a shell that stays, and output that
//! stays addressable after the call that produced it has returned.
//!
//! `TerminalClose` is a fourth tool beyond the three the issue names. Without
//! it the cap in `terminal_registry` is a trap: an agent that opens the limit
//! has no way to free a slot, and would be told to close a terminal it cannot
//! close.

use serde_json::json;

use crate::terminal_registry as registry;
use crate::terminal_shell as shells;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult, WorkingTreeEffect};

/// Most output one `TerminalRead` returns.
///
/// Sized against the 24 KB replay budget a shell result gets, so an ordinary
/// read stays whole in context. A larger one is not lost: the read reports what
/// it left behind, and the next call resumes there.
const MAX_READ_BYTES: usize = 16_000;

/// Opens a shell that outlives the call.
pub struct TerminalCreateTool;

#[async_trait::async_trait]
impl Tool for TerminalCreateTool {
    fn name(&self) -> &str {
        "TerminalCreate"
    }

    fn description(&self) -> &str {
        "Start a persistent shell and return its id. Unlike Bash, the shell \
         stays alive between calls, so a directory change, an activated \
         environment or an exported variable is still in effect for the next \
         TerminalWrite. Use it for interactive programs, for long-running \
         processes you want to check on later, and for any sequence where one \
         command depends on the state the last one left. Feed the id to \
         TerminalWrite and TerminalRead, and call TerminalClose when done."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "shell": {
                    "type": "string",
                    "enum": shells::SHELLS,
                    "description": format!(
                        "Which shell to run (default {}). \"cmd\" is Windows only.",
                        shells::default_shell()
                    )
                },
                "cwd": {
                    "type": "string",
                    "description": "Directory to start in (default: the session working directory)"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let shell = string_arg(&input, "shell").unwrap_or_else(|| shells::default_shell().into());
        let cwd = string_arg(&input, "cwd").map_or_else(
            || ctx.working_dir.clone(),
            |value| ctx.working_dir.join(value),
        );
        let program = match shells::build(&shell, &cwd) {
            Ok(program) => program,
            Err(error) => return ToolResult::error(error),
        };

        let id = format!("term-{}", uuid::Uuid::new_v4().simple());
        match registry::create(&ctx.session_id, id.clone(), shell.clone(), program) {
            Ok(_) => ToolResult::success(format!(
                "Terminal {id} is running {shell} in {}.\n\
                 Write to it with TerminalWrite, then read with TerminalRead \
                 (start at offset 0).",
                cwd.display()
            )),
            Err(error) => ToolResult::error(error),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        // Opening a shell runs the user's startup files, which can do anything
        // a command could.
        PermissionLevel::Risky
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::Arbitrary
    }
}

/// Sends input to a live terminal.
///
/// Carries the operator's command lists so the text being written is classified
/// exactly as `Bash` would classify it. Without that this tool would be a way
/// to run anything at `Risky` — a hole shaped like a feature.
#[derive(Default)]
pub struct TerminalWriteTool {
    pub safe_commands: Vec<String>,
    pub risky_commands: Vec<String>,
    pub dangerous_commands: Vec<String>,
}

#[async_trait::async_trait]
impl Tool for TerminalWriteTool {
    fn name(&self) -> &str {
        "TerminalWrite"
    }

    fn description(&self) -> &str {
        "Send text to a terminal opened by TerminalCreate. A trailing newline \
         is added unless you set newline to false, so ordinary commands run as \
         written. Returns immediately — the command keeps running in the \
         terminal — so call TerminalRead afterwards to see what it produced. \
         Set newline to false to answer a prompt without submitting, or to send \
         a control character."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from TerminalCreate"},
                "text": {"type": "string", "description": "Text to send"},
                "newline": {
                    "type": "boolean",
                    "description": "Append a newline so the shell runs it (default true)"
                }
            },
            "required": ["id", "text"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let Some(id) = string_arg(&input, "id") else {
            return ToolResult::error("id is required");
        };
        let Some(text) = string_arg(&input, "text") else {
            return ToolResult::error("text is required");
        };
        let Some(terminal) = registry::get(&id) else {
            return ToolResult::error(unknown_terminal(&id));
        };

        // Where the caller resumes reading. Taken before the write so output
        // the command produces immediately cannot land in the gap.
        let offset = terminal.produced();
        let newline = input
            .get("newline")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        terminal.write(&if newline {
            format!("{text}\n")
        } else {
            text.clone()
        });

        ToolResult::success(format!(
            "Sent to {id}. Read the result with TerminalRead at offset {offset}."
        ))
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        let text = input
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        // Classified by the same rules as `Bash`, then floored at `Risky`: a
        // command the operator marked safe is still being typed into a live
        // shell whose state this call cannot see.
        match archon_permissions::classifier::classify_command(
            text,
            &self.safe_commands,
            &self.risky_commands,
            &self.dangerous_commands,
        ) {
            archon_permissions::classifier::CommandClass::Dangerous => PermissionLevel::Dangerous,
            _ => PermissionLevel::Risky,
        }
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::Arbitrary
    }
}

/// Reads accumulated output from a live terminal.
pub struct TerminalReadTool;

#[async_trait::async_trait]
impl Tool for TerminalReadTool {
    fn name(&self) -> &str {
        "TerminalRead"
    }

    fn description(&self) -> &str {
        "Read output a terminal has produced since a byte offset. Start at 0, \
         then pass back the next_offset from the previous read to see only what \
         is new — so a long-running process can be left alone and checked on \
         later. Escape sequences are stripped. Large reads are truncated and \
         report how much is left."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from TerminalCreate"},
                "since": {
                    "type": "integer",
                    "description": "Byte offset to resume from (default 0, the start)"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let Some(id) = string_arg(&input, "id") else {
            return ToolResult::error("id is required");
        };
        let Some(terminal) = registry::get(&id) else {
            return ToolResult::error(unknown_terminal(&id));
        };

        let since = input
            .get("since")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let read = terminal.read(since, MAX_READ_BYTES);

        let mut out = String::new();
        if read.dropped > 0 {
            out.push_str(&format!(
                "[{} bytes scrolled out of the buffer before this point]\n",
                read.dropped
            ));
        }
        out.push_str(&read.text);
        if read.remaining > 0 {
            out.push_str(&format!(
                "\n[{} more bytes available; read again from {}]",
                read.remaining, read.next_offset
            ));
        }
        if out.is_empty() {
            out.push_str("[no new output]");
        }
        out.push_str(&format!("\n\nnext_offset: {}", read.next_offset));
        ToolResult::success(out)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        // Reading back output this agent already caused has no effect of its
        // own, and gating it would mean a prompt every time a long command is
        // checked on — which is the thing this phase exists to make possible.
        PermissionLevel::Safe
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::None
    }
}

/// Ends a terminal and its process.
pub struct TerminalCloseTool;

#[async_trait::async_trait]
impl Tool for TerminalCloseTool {
    fn name(&self) -> &str {
        "TerminalClose"
    }

    fn description(&self) -> &str {
        "Close a terminal and kill whatever is running in it. Do this as soon \
         as a terminal is no longer needed: only a small number can be open at \
         once, and any still open are closed when the session ends."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from TerminalCreate"}
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let Some(id) = string_arg(&input, "id") else {
            return ToolResult::error("id is required");
        };
        if registry::close(&id) {
            ToolResult::success(format!("Terminal {id} is closed."))
        } else {
            ToolResult::error(unknown_terminal(&id))
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        // Ending something this agent started. Refusing to let it tidy up would
        // leave shells running instead.
        PermissionLevel::Safe
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::None
    }
}

/// Close every terminal one session opened, and report how many there were.
///
/// The way out. The cap and the idle timeout both leave a recently-used
/// terminal running, which is exactly the state one is in when the user quits,
/// so the session-end path has to close them explicitly. Exposed here rather
/// than from the registry so the registry itself stays crate-private.
pub fn close_session_terminals(session_id: &str) -> usize {
    registry::close_session(session_id)
}

/// One message for the one mistake there is to make with an id.
///
/// Says why it might be gone, because "no such terminal" alone reads as a bad
/// id when the likeliest cause is that it was reaped or the session moved on.
fn unknown_terminal(id: &str) -> String {
    format!(
        "no terminal {id}. It may have been closed, or reaped after being idle. \
         Open a new one with TerminalCreate."
    )
}

fn string_arg(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "terminal_tools_tests.rs"]
mod tests;
