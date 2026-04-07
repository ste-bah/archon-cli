//! Chapter writer — placeholder generation (LLM integration deferred).
//!
//! Full LLM-powered synthesis will be wired in during hardening.
//! For now this module provides a pass-through formatter.
//!
//! TODO(REQ-RESEARCH-007): Replace with LLM-driven chapter synthesis
//! that uses `LlmClient` for coherent academic prose generation.

/// Write a chapter from mapped content. Currently a pass-through that
/// wraps the source content under an H2 heading.
///
/// # Tech Debt
///
/// This is intentionally a pass-through. Full LLM synthesis requires
/// the `LlmClient` plumbing which is not yet available in the final
/// stage pipeline.
pub fn synthesize_chapter(title: &str, source_content: &str) -> String {
    format!("## {}\n\n{}", title, source_content)
}
