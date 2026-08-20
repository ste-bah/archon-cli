//! Carry an agent's originating task across a compaction.
//!
//! Split from `compact.rs` for the 500-line ceiling; the two are one unit and
//! both compaction paths call in here.

use crate::messages::ContextMessage;

/// Maximum characters of the originating task carried across a compaction.
///
/// Long enough for any realistic agent brief, short enough that a pasted file
/// in the first message cannot eat the window the compaction just reclaimed.
pub const MAX_PRESERVED_TASK_CHARS: usize = 4_000;

/// Build a summary header that restates the originating task above the summary.
///
/// WHY: `compact_messages` keeps only the tail plus a summary. A subagent is
/// given its task exactly once, as `messages[0]` (`SubagentRunner::run`), so a
/// successful compaction deleted the only statement of what it was doing and
/// left the summariser's own scaffolding — "## Current State", "use bullet
/// points" — as the sole instruction in context. Branches then answered the
/// scaffolding: an acceptance-evidence audit returned a bullet-point context
/// summary, and the gate accepted it because the envelope was well-formed.
///
/// The defect was latent for as long as subagent compaction always failed on
/// an empty summary from the reasoning model; routing it to a summarising
/// model made it fire. Restating the task costs a few hundred tokens and
/// removes the failure mode for every agent that shares this path.
pub fn build_structured_summary_header_with_task(task: &str, summary_text: &str) -> String {
    prepend_task_block(
        task,
        &crate::compact::build_structured_summary_header(summary_text),
    )
}

/// Opening delimiter of a restated task block.
const TASK_BLOCK_OPEN: &str =
    "[Original Task — still in force, restated verbatim after compaction]";
/// Closing delimiter. Present so a second compaction can recognise a head it
/// wrote itself and unwrap it, instead of treating the whole header as the task
/// and nesting another block around it — which accumulated a stale summary per
/// round until the 4k budget was spent on dead context.
const TASK_BLOCK_CLOSE: &str = "[/Original Task]";

/// Put the restated task above `body`, whatever shape `body` has.
///
/// Kept separate so the micro path kernel can reuse it without inheriting the
/// structured `## Key Decisions` header, which downstream consumers parse.
pub(crate) fn prepend_task_block(task: &str, body: &str) -> String {
    let mut header = String::with_capacity(task.len() + body.len() + 128);
    header.push_str(TASK_BLOCK_OPEN);
    header.push('\n');
    header.push_str(task);
    header.push('\n');
    header.push_str(TASK_BLOCK_CLOSE);
    header.push_str("\n\n");
    header.push_str(body);
    header
}

/// The task carried by a head this module wrote on an earlier compaction.
///
/// Returns `None` for any text that is not a task block, so an ordinary first
/// message falls through to being restated whole.
/// Matched on the full structural terminator — newline, delimiter, blank line —
/// and on the LAST one, so a task that merely mentions `[/Original Task]` mid
/// sentence is not cut short at its own words. `rfind` because the block we
/// wrote always terminates immediately before the body.
fn unwrap_task_block(text: &str) -> Option<&str> {
    const TERMINATOR: &str = "\n[/Original Task]\n\n";
    debug_assert!(TERMINATOR.contains(TASK_BLOCK_CLOSE));
    let rest = text.strip_prefix(TASK_BLOCK_OPEN)?.strip_prefix('\n')?;
    let end = rest.rfind(TERMINATOR)?;
    Some(&rest[..end])
}

/// The originating task, when the split would otherwise discard it.
///
/// `None` when the head is already inside the retained tail, when the first
/// message is not a user message, or when it carries no text to restate.
pub(crate) fn preserved_task(messages: &[ContextMessage], split_point: usize) -> Option<String> {
    if split_point == 0 {
        return None;
    }
    let first = messages.first()?;
    if first.role != "user" {
        return None;
    }
    let text = message_text(first);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // A head this module wrote on an earlier compaction already carries the
    // task; carry that forward rather than the whole header, or every round
    // wraps the previous round and the summaries pile up.
    let task = unwrap_task_block(trimmed).unwrap_or(trimmed);
    if task.is_empty() {
        return None;
    }
    Some(truncate_on_char_boundary(task, MAX_PRESERVED_TASK_CHARS))
}

/// Readable text of a message, whether its content is a string or blocks.
///
/// Tool-use and tool-result blocks are skipped: a seed prompt carries neither,
/// and replaying one into the header would reintroduce an unpaired block.
fn message_text(message: &ContextMessage) -> String {
    match &message.content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("text"))
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Truncate on a char boundary, saying so where it happened.
fn truncate_on_char_boundary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("\n[task text truncated]");
    out
}
