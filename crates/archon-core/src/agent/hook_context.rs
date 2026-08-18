//! Context contributed by lifecycle hooks (#187).
//!
//! `AggregatedHookResult.additional_contexts` was aggregated by the registry
//! for every event and read by exactly one caller — `PostToolUse`. A hook that
//! published, say, a project's working agreements at `SessionStart` ran,
//! produced correct output, was merged into the aggregate, and changed nothing
//! about what the model saw.
//!
//! The accessors and the injector live together here rather than split across
//! `lifecycle` and `memory_integration`: they are one feature, and the write
//! side is meaningless without the read side that renders it.

use super::*;

impl Agent {
    /// Record context contributed by a lifecycle hook.
    ///
    /// Appends rather than replaces: several hooks may fire for one event, and
    /// `PostCompact` adds to what `SessionStart` established rather than
    /// superseding it. Blank contributions are dropped so an empty hook cannot
    /// leave a stray block in the prompt.
    pub fn add_hook_session_context(&mut self, contexts: Vec<String>) {
        self.hook_session_context
            .extend(contexts.into_iter().filter(|c| !c.trim().is_empty()));
    }

    /// Context contributed so far by lifecycle hooks.
    pub fn hook_session_context(&self) -> &[String] {
        &self.hook_session_context
    }

    /// Inject hook-contributed context into this turn's system prompt.
    ///
    /// Appended after the cached blocks, like the critical reminder, so a hook
    /// firing mid-session cannot invalidate the stable prefix. Re-injected
    /// every turn rather than prepended once, which is what makes it survive
    /// compaction — the case that matters for long sessions.
    pub(super) fn inject_hook_session_context(&self, system: &mut Vec<serde_json::Value>) {
        if self.hook_session_context.is_empty() {
            return;
        }
        system.push(serde_json::json!({
            "type": "text",
            "text": format!(
                "<hook-context>\n{}\n</hook-context>",
                self.hook_session_context.join("\n")
            ),
        }));
    }
}
