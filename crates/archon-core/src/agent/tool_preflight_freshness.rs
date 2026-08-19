//! Read-before-write, checked before a mutating tool runs (#193 Phase A).
//!
//! Deliberately here and not inside `file_edit.rs`. The tool calls the
//! filesystem and knows nothing about policy; the dispatcher consults policy
//! and then calls the tool. Delete this step and an unconstrained-but-working
//! filesystem is what remains, which is the property `read_before_edit = "off"`
//! has to be able to promise honestly.
//!
//! It sits alongside the permission gate and the cognitive gate in
//! `preflight_single_tool`, which is where every other "may this run" question
//! is already asked.

use archon_tools::file_observation::{FILE_OBSERVATIONS, Observer, Verdict};

use crate::config::ReadBeforeEdit;

use super::*;

/// Tools whose call is a write to a path named in their input.
///
/// `Bash` is not here on purpose. A shell command can write anywhere and names
/// no path to check, so demanding a prior read of something it never mentions
/// would refuse work it cannot describe. The guarantee is deliberately partial
/// and honest about it rather than broad and unenforceable.
const GUARDED_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit"];

/// Tools that show an agent the current contents of a path.
const OBSERVING_TOOLS: &[&str] = &["Read", "NotebookRead", "Grep"];

/// Why this write should not proceed, or `None` to let it through.
///
/// A free function rather than an `Agent` method because there are two tool
/// loops — the parent's and the subagent runner's — and a policy that only one
/// of them consulted would be a guarantee with a hole in the middle exactly
/// where Archon runs the most agents.
///
/// Under `Warn` this logs and returns `None`: the write happens, and the reason
/// is on the record.
pub(crate) fn refusal_for(
    config: crate::config::FilesystemConfig,
    observer: &Observer,
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<String> {
    if !config.enforces_freshness() || !GUARDED_TOOLS.contains(&tool_name) {
        return None;
    }
    let path = guarded_path(input)?;
    let reason = refusal(
        tool_name,
        &path,
        &FILE_OBSERVATIONS.verdict(observer, &path),
    )?;

    if config.read_before_edit == ReadBeforeEdit::Warn {
        tracing::warn!(tool = %tool_name, path = %path.display(), "{reason}");
        return None;
    }
    Some(reason)
}

/// Record what a tool just showed this agent.
///
/// A failed read observes nothing: the agent did not see the file, and
/// pretending otherwise would licence an edit on bytes nobody looked at.
pub(crate) fn record(
    config: crate::config::FilesystemConfig,
    observer: &Observer,
    tool_name: &str,
    input: &serde_json::Value,
    succeeded: bool,
) {
    if !config.enforces_freshness() || !succeeded {
        return;
    }
    if OBSERVING_TOOLS.contains(&tool_name) {
        for path in observed_paths(tool_name, input) {
            FILE_OBSERVATIONS.record(observer, &path);
        }
    }
    // An agent's own write is also an observation, or its second edit to a file
    // would be refused by its first.
    if GUARDED_TOOLS.contains(&tool_name)
        && let Some(path) = guarded_path(input)
    {
        FILE_OBSERVATIONS.record(observer, &path);
    }
}

/// Who is doing the looking, for a given tool invocation.
pub(crate) fn observer_for(ctx: &archon_tools::tool::ToolContext) -> Observer {
    Observer::new(&ctx.session_id, ctx.subagent_id.as_deref())
}

impl Agent {
    /// Refuse a write this agent has no fresh reading behind.
    ///
    /// Returns `true` when the tool may proceed. A refusal is reported as a
    /// tool result rather than an error so the model can read the reason and
    /// do the obvious thing about it.
    pub(super) async fn freshness_allows_tool(
        &mut self,
        tool: &PendingToolCall,
        input: &serde_json::Value,
    ) -> bool {
        let observer = self.observer();
        let Some(reason) = refusal_for(self.config.filesystem, &observer, &tool.name, input) else {
            return true;
        };

        let result = ToolResult::error(reason);
        self.send_event(AgentEvent::ToolCallComplete {
            name: tool.name.clone(),
            id: tool.id.clone(),
            result: result.clone(),
            transcript_summary: None,
        })
        .await;
        self.state.add_tool_result(&tool.id, &result.content, true);
        false
    }

    /// Record what a tool just showed this agent.
    pub(super) fn record_observation(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        succeeded: bool,
    ) {
        record(
            self.config.filesystem,
            &self.observer(),
            tool_name,
            input,
            succeeded,
        );
    }

    /// `session_id` is copied verbatim into subagents, so it cannot separate a
    /// child from its parent; the subagent id is what does.
    fn observer(&self) -> Observer {
        Observer::new(&self.config.session_id, self.config.subagent_id.as_deref())
    }
}

/// The message for a verdict that should stop the write, or `None` to proceed.
fn refusal(tool: &str, path: &std::path::Path, verdict: &Verdict) -> Option<String> {
    let shown = path.display();
    match verdict {
        Verdict::Fresh => None,
        Verdict::Unobserved => Some(format!(
            "{tool} was refused: you have not read {shown} in this session, so the \
             text you are replacing may not be what is in the file. Read it first, \
             then edit."
        )),
        Verdict::Stale { detail } => Some(format!(
            "{tool} was refused: {shown} is not what you read — {detail}. Read it \
             again before editing, because the text you are replacing may now mean \
             something different."
        )),
    }
}

/// The path a guarded tool is about to write.
fn guarded_path(input: &serde_json::Value) -> Option<std::path::PathBuf> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("notebook_path"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
}

/// The paths an observing tool showed.
///
/// `Grep` reports matches with their file names, and a match is a real sighting
/// of those bytes — enough to choose an `old_string` from, and still subject to
/// the freshness check afterwards. It observes the path it searched only when
/// that path is a single file; a directory search shows lines, not files, and
/// treating a whole tree as read would hand back the guarantee.
fn observed_paths(tool_name: &str, input: &serde_json::Value) -> Vec<std::path::PathBuf> {
    let candidate = if tool_name == "Grep" {
        input.get("path")
    } else {
        input
            .get("file_path")
            .or_else(|| input.get("path"))
            .or_else(|| input.get("notebook_path"))
    };

    candidate
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| tool_name != "Grep" || path.is_file())
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(key: &str, value: &str) -> serde_json::Value {
        serde_json::json!({ key: value })
    }

    #[test]
    fn only_the_writing_tools_are_guarded() {
        assert!(GUARDED_TOOLS.contains(&"Edit"));
        assert!(GUARDED_TOOLS.contains(&"Write"));
        assert!(
            !GUARDED_TOOLS.contains(&"Bash"),
            "a shell command names no path to check, so guarding it would refuse \
             work it cannot describe"
        );
    }

    #[test]
    fn the_written_path_is_read_from_any_of_the_field_names() {
        assert_eq!(
            guarded_path(&input("file_path", "/a/b.rs")),
            Some(std::path::PathBuf::from("/a/b.rs"))
        );
        assert_eq!(
            guarded_path(&input("notebook_path", "/a/b.ipynb")),
            Some(std::path::PathBuf::from("/a/b.ipynb"))
        );
        assert_eq!(guarded_path(&serde_json::json!({})), None);
        assert_eq!(guarded_path(&input("file_path", "  ")), None);
    }

    /// A directory grep shows lines, not files. Recording the tree as read
    /// would licence an edit to any file in it.
    #[test]
    fn a_directory_grep_observes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().display().to_string();
        assert!(observed_paths("Grep", &input("path", &path)).is_empty());
    }

    #[test]
    fn a_single_file_grep_observes_that_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn main() {}").expect("write");

        assert_eq!(
            observed_paths("Grep", &input("path", &file.display().to_string())),
            vec![file]
        );
    }

    #[test]
    fn a_read_observes_its_path() {
        assert_eq!(
            observed_paths("Read", &input("file_path", "/a/b.rs")),
            vec![std::path::PathBuf::from("/a/b.rs")]
        );
    }

    /// The message has to say what to do, not just that something is wrong.
    #[test]
    fn the_refusals_say_what_to_do_about_it() {
        let path = std::path::Path::new("/a/b.rs");

        assert_eq!(refusal("Edit", path, &Verdict::Fresh), None);

        let unobserved = refusal("Edit", path, &Verdict::Unobserved).expect("refused");
        assert!(unobserved.contains("have not read"), "{unobserved}");
        assert!(unobserved.contains("Read it first"), "{unobserved}");
        assert!(unobserved.contains("/a/b.rs"), "{unobserved}");

        let stale = refusal(
            "Write",
            path,
            &Verdict::Stale {
                detail: "it has been modified since you read it".into(),
            },
        )
        .expect("refused");
        assert!(stale.contains("modified since"), "{stale}");
        assert!(stale.contains("Read it again"), "{stale}");
    }
}
