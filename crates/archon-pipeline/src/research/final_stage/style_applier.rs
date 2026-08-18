//! Style applier — applies a style profile to the final paper.
//!
//! No style profiles are implemented yet. What this module exists to prevent
//! is the failure that was actually shipping: `FinalStageOrchestrator::run`
//! never called it, so `FinalStageOptions::style_profile_id` was read by
//! nothing at all. Naming a profile produced an unstyled paper and no
//! indication why — a knob wired to nothing. `run` now calls this on every
//! run and puts the returned warning in `FinalStageResult::warnings`.
//!
//! TODO(REQ-RESEARCH-007): Implement LLM-driven style application using
//! defined style profiles (British English, APA formatting, etc.).

/// Apply a style profile to the final paper text.
///
/// Returns the paper, plus a warning when a profile was requested that cannot
/// be honoured. The paper is always returned: an unimplemented profile is not
/// a reason to fail a run that has already assembled a document, and
/// `FinalStageError::StyleError` is reserved for a profile that fails while
/// being applied.
pub fn apply_style(paper: &str, style_profile_id: Option<&str>) -> (String, Option<String>) {
    match style_profile_id {
        None => (paper.to_string(), None),
        Some(id) => {
            tracing::warn!(
                profile = id,
                "style profile requested but none are implemented; paper left unstyled"
            );
            (
                paper.to_string(),
                Some(format!(
                    "style profile {id:?} was requested but no style profiles are implemented \
                     (REQ-RESEARCH-007); the paper is unstyled"
                )),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_profile_returns_the_paper_and_no_warning() {
        let (out, warning) = apply_style("# Paper\n", None);
        assert_eq!(out, "# Paper\n");
        assert!(warning.is_none(), "unexpected warning: {warning:?}");
    }

    #[test]
    fn a_requested_profile_is_reported_rather_than_silently_dropped() {
        let (out, warning) = apply_style("# Paper\n", Some("apa"));
        assert_eq!(
            out, "# Paper\n",
            "the paper must survive an unknown profile"
        );
        let warning = warning.expect("requesting a profile must produce a warning");
        assert!(
            warning.contains("apa"),
            "the warning must name the profile that was ignored: {warning}"
        );
    }
}
