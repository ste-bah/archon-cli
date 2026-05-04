//! Shared LEANN semantic search interface for pipeline memory fallbacks.

/// Trait for LEANN semantic search fallback when a memory key is missing.
pub trait LeannSearcher: Send + Sync {
    /// Search the LEANN index for content matching the given query.
    fn search(&self, query: &str) -> String;
}
