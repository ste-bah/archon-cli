//! Section writer — synthesizes section content from mapped specialist outputs.
//!
//! Currently a pass-through stub. Full LLM-driven section synthesis will be
//! wired during Phase 5 hardening.

use std::collections::BTreeMap;

use super::super::sections::SectionType;
use super::mapper::SectionAssignment;

/// Content for a single report section, ready for combination.
#[derive(Debug, Clone)]
pub struct SectionContent {
    /// The section type.
    pub section: SectionType,
    /// Synthesized content for this section.
    pub content: String,
    /// Agent keys that contributed to this section (for provenance).
    pub contributors: Vec<String>,
}

/// Provider that synthesizes section content from assignments.
///
/// Currently a pass-through: content from all contributing specialists is
/// concatenated under the section title.
///
/// Phase 5 will replace this with LLM-driven synthesis.
pub struct SectionWriterProvider;

impl SectionWriterProvider {
    /// Synthesize all sections from the given assignments.
    ///
    /// Assignments are grouped by section, and content from all contributors
    /// is concatenated. Sections are emitted in display order.
    pub fn synthesize_sections(&self, assignments: &[SectionAssignment]) -> Vec<SectionContent> {
        let mut by_section: BTreeMap<SectionType, Vec<&SectionAssignment>> = BTreeMap::new();

        for assignment in assignments {
            by_section.entry(assignment.section).or_default().push(assignment);
        }

        let mut sections = Vec::new();
        for section_type in SectionType::all_ordered() {
            if let Some(assigns) = by_section.get(&section_type) {
                let contributors: Vec<String> = assigns
                    .iter()
                    .map(|a| a.agent_key.clone())
                    .collect();

                let body = assigns
                    .iter()
                    .map(|a| a.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");

                let content = format!("## {}\n\n{}", section_type.title(), body);

                sections.push(SectionContent {
                    section: section_type,
                    content,
                    contributors,
                });
            }
        }

        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::mapper::SectionAssignment;

    #[test]
    fn test_synthesize_sections_groups_by_section() {
        let assignments = vec![
            SectionAssignment {
                section: SectionType::EquilibriumAnalysis,
                agent_key: "nash-equilibrium-finder".to_string(),
                content: "Nash eq found.".to_string(),
            },
            SectionAssignment {
                section: SectionType::Recommendations,
                agent_key: "strategic-recommendations".to_string(),
                content: "Cooperate on first move.".to_string(),
            },
        ];

        let writer = SectionWriterProvider;
        let sections = writer.synthesize_sections(&assignments);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].section, SectionType::EquilibriumAnalysis);
        assert_eq!(sections[1].section, SectionType::Recommendations);
        assert!(sections[0].content.contains("Nash eq found."));
        assert!(sections[1].content.contains("Cooperate on first move."));
    }
}
