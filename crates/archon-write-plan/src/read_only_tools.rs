//! The read-only tool vocabulary: the native tool names an agent may hold
//! without being able to change anything.
//!
//! Two layers read this list and must never drift. The pipeline adapter
//! (`archon_pipeline::subagent_adapter`) hands it to a `ReadOnly` agent as
//! its default tool set. The workflow host's native-tool admission
//! (`workflow_live_mcp::declared_native_tools`, Issue-28) lets a task declare
//! a native tool by name and admits it to a read-only stage only if it is
//! already here — the same list that bounds the stage when nothing is
//! declared, so a declaration can widen a reviewer's vocabulary but never its
//! reach.
//!
//! It lives in this leaf for the reason the overlap table does (see the crate
//! doc): the admission is `archon-workflow` code, the default is
//! `archon-pipeline` code, and `archon-workflow` cannot depend on
//! `archon-pipeline`. A copy on each side is two opinions; a table both
//! import is one.

/// Every tool a `ReadOnly` agent is offered by default.
pub const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "WebSearch",
    "WebFetch",
    "DocList",
    "DocGet",
    "DocStatus",
    "DocSearch",
    "DocAnswer",
    "DocProvenance",
    "DocInspect",
    "DocModelStatus",
    "memory_recall",
    "LeannSearch",
    "LeannFindSimilar",
    "lsp",
    "CartographerScan",
    "ToolSearch",
    "AgentCatalog",
];

/// Whether `name` is in the read-only vocabulary.
pub fn is_read_only_tool(name: &str) -> bool {
    READ_ONLY_TOOLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_holds_readers_and_no_writers() {
        assert!(is_read_only_tool("Read"));
        assert!(is_read_only_tool("memory_recall"));
        for writer in [
            "Write",
            "Edit",
            "ApplyPatch",
            "Bash",
            "memory_store",
            "TodoWrite",
        ] {
            assert!(!is_read_only_tool(writer), "{writer} must not be read-only");
        }
    }

    #[test]
    fn membership_is_exact() {
        assert!(!is_read_only_tool("read"));
        assert!(!is_read_only_tool(" Read"));
        assert!(!is_read_only_tool(""));
    }
}
