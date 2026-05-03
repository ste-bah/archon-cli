//! Style applier — applies a style profile to the final report.
//!
/// Apply a style profile to the report text.
///
/// When `style_profile_id` is `None`, the report is returned unchanged.
pub fn apply_style(report: &str, style_profile_id: Option<&str>) -> String {
    let Some(style) = style_profile_id else {
        return report.to_string();
    };

    let guidance = match style {
        "executive" => "Style: executive brief. Prioritise decisions, risks, and next actions.",
        "academic" => {
            "Style: academic. Prioritise definitions, assumptions, and methodological caveats."
        }
        "technical" => {
            "Style: technical. Prioritise mechanisms, models, and implementation constraints."
        }
        other => return format!("<!-- style: {other} -->\n\n{report}"),
    };

    format!("<!-- {guidance} -->\n\n{report}")
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
    fn test_apply_style_with_profile_id_marks_report() {
        let input = "# Report\n\nContent.";
        let output = apply_style(input, Some("academic"));
        assert!(output.contains("Style: academic"));
        assert!(output.contains(input));
    }
}
