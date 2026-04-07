//! Style applier — applies style profile to final paper output.
//!
//! Currently a pass-through. Full style application will be implemented
//! during hardening when style profiles are fully defined.
//!
//! TODO(REQ-RESEARCH-007): Implement LLM-driven style application using
//! defined style profiles (British English, APA formatting, etc.).

/// Apply a style profile to the final paper text.
///
/// When `_style_profile_id` is `None` or the profile is not yet implemented,
/// the paper is returned unchanged.
///
/// # Tech Debt
///
/// This is intentionally a pass-through. Full style application requires
/// the `LlmClient` plumbing and defined style profile schemas.
pub fn apply_style(paper: &str, _style_profile_id: Option<&str>) -> String {
    paper.to_string()
}
