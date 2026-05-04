//! Section combiner — merges sections into a final report with provenance footnotes.

use super::writer::SectionContent;

/// Combine sections into a final game-theory report.
///
/// Each section includes a provenance footnote listing the contributing
/// specialist agents. Sections are emitted in their natural display order.
pub fn combine_sections(sections: &[SectionContent]) -> String {
    let mut report = String::new();

    // Title
    report.push_str("# Strategic Game-Theory Analysis\n\n");

    // Table of Contents
    report.push_str("## Contents\n\n");
    for sec in sections {
        report.push_str(&format!(
            "- [{}](#{})\n",
            sec.section.title(),
            anchor(sec.section.title())
        ));
    }
    report.push('\n');

    report.push_str("## Fingerprint Summary\n\n");
    report.push_str(
        "See the `9-Axis Fingerprint` section for the persisted Tier 1 strategic fingerprint.\n\n",
    );

    // Sections in order
    for sec in sections {
        report.push_str(&format!("{}\n", sec.content));

        // Provenance footnote
        let contributors = sec
            .contributors
            .iter()
            .map(|c| format!("`{}`", c))
            .collect::<Vec<_>>()
            .join(", ");
        report.push_str(&format!(
            "\n*Provenance: {} — contributed by {contributors}.*\n\n",
            sec.section.title(),
        ));

        report.push_str("---\n\n");
    }

    report
}

fn anchor(title: &str) -> String {
    title.to_lowercase().replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::super::super::sections::SectionType;
    use super::super::writer::SectionContent;
    use super::*;

    #[test]
    fn test_combine_sections_includes_provenance() {
        let sections = vec![
            SectionContent {
                section: SectionType::ExecutiveSummary,
                content: "## Executive Summary\n\nOverview.".to_string(),
                contributors: vec!["game-tree-archaeologist".to_string()],
            },
            SectionContent {
                section: SectionType::Recommendations,
                content: "## Recommendations\n\nCooperate.".to_string(),
                contributors: vec!["strategic-recommendations".to_string()],
            },
        ];

        let report = combine_sections(&sections);

        assert!(report.contains("Strategic Game-Theory Analysis"));
        assert!(report.contains("Executive Summary"));
        assert!(report.contains("Recommendations"));
        assert!(report.contains("*Provenance: Executive Summary"));
        assert!(report.contains("`game-tree-archaeologist`"));
        assert!(report.contains("*Provenance: Recommendations"));
        assert!(report.contains("`strategic-recommendations`"));
    }

    #[test]
    fn test_combine_empty_sections() {
        let report = combine_sections(&[]);
        assert!(report.contains("Strategic Game-Theory Analysis"));
        assert!(report.contains("## Contents"));
    }

    #[test]
    fn test_combine_sections_has_table_of_contents() {
        let sections = vec![SectionContent {
            section: SectionType::PayoffAndStrategyStructure,
            content: "## Payoff and Strategy Structure\n\nAnalysis text.".to_string(),
            contributors: vec!["payoff-matrix-builder".to_string()],
        }];

        let report = combine_sections(&sections);
        assert!(report.contains("[Payoff and Strategy Structure]"));
        assert!(report.contains("(#payoff-and-strategy-structure)"));
    }

    #[test]
    fn test_combiner_outputs_all_11_sections_in_order() {
        let sections = SectionType::all_ordered()
            .into_iter()
            .map(|section| SectionContent {
                section,
                content: format!("## {}\n\nBody.", section.title()),
                contributors: vec!["game-classifier".to_string()],
            })
            .collect::<Vec<_>>();

        let report = combine_sections(&sections);
        let mut last_pos = 0usize;
        for section in SectionType::all_ordered() {
            let heading = format!("## {}", section.title());
            let pos = report
                .find(&heading)
                .unwrap_or_else(|| panic!("missing {heading}"));
            assert!(pos >= last_pos, "{heading} emitted out of order");
            last_pos = pos;
        }
    }

    #[test]
    fn test_combiner_includes_toc_fingerprint_summary_provenance_footer() {
        let sections = SectionType::all_ordered()
            .into_iter()
            .map(|section| SectionContent {
                section,
                content: format!("## {}\n\nBody.", section.title()),
                contributors: vec!["game-classifier".to_string()],
            })
            .collect::<Vec<_>>();

        let report = combine_sections(&sections);

        assert!(report.contains("## Contents"));
        assert!(report.contains("## Fingerprint Summary"));
        assert!(report.contains("## Provenance Footer"));
    }
}
