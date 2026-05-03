//! Style applier — applies a style profile to the final report.
//!
//! Currently a pass-through. Full style application (tone, formatting,
//! citation style) will be implemented during Phase 5 hardening.

/// Apply a style profile to the report text.
///
/// When `_style_profile_id` is `None`, the report is returned unchanged.
///
/// Phase 5 will add support for profiles like "academic", "executive",
/// and "military-brief".
pub fn apply_style(report: &str, _style_profile_id: Option<&str>) -> String {
    report.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_style_pass_through() {
        let input = "# Report\n\nContent.";
        let output = apply_style(input, None);
        assert_eq!(input, output);
    }

    #[test]
    fn test_apply_style_with_profile_id_is_still_pass_through() {
        let input = "# Report\n\nContent.";
        let output = apply_style(input, Some("academic"));
        assert_eq!(input, output);
    }
}
