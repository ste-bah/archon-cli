//! Section mapper — assigns specialist outputs to report sections using the
//! deterministic mapping from [`super::super::sections::sections_for_specialist`].

use super::super::sections::{sections_for_specialist, SectionType};
use super::scanner::SpecialistOutput;

/// A specialist output assigned to a specific section.
#[derive(Debug, Clone)]
pub struct SectionAssignment {
    /// The target section.
    pub section: SectionType,
    /// The agent key that produced this content.
    pub agent_key: String,
    /// The content to include in this section.
    pub content: String,
}

/// Map scanned specialist outputs to their target sections.
///
/// Uses the deterministic `sections_for_specialist` mapping. Each output
/// is assigned to every section its specialist contributes to.
pub fn map_to_sections(outputs: &[SpecialistOutput]) -> Vec<SectionAssignment> {
    let mut assignments = Vec::new();

    for output in outputs {
        let target_sections = sections_for_specialist(&output.agent_key);
        for section in target_sections {
            assignments.push(SectionAssignment {
                section,
                agent_key: output.agent_key.clone(),
                content: output.content.clone(),
            });
        }
    }

    assignments
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::scanner::SpecialistOutput;

    #[test]
    fn test_map_known_agent_to_correct_sections() {
        let outputs = vec![SpecialistOutput {
            agent_key: "nash-equilibrium-finder".to_string(),
            content: "Nash eq at (A,B).".to_string(),
            quality_checks: vec![],
        }];

        let assignments = map_to_sections(&outputs);

        // nash-equilibrium-finder → EquilibriumAnalysis + FormalAnalysis
        let sections: Vec<SectionType> = assignments.iter().map(|a| a.section).collect();
        assert!(sections.contains(&SectionType::EquilibriumAnalysis));
        assert!(sections.contains(&SectionType::FormalAnalysis));
        assert_eq!(assignments.len(), 2, "two sections for nash-equilibrium-finder");
        for a in &assignments {
            assert_eq!(a.agent_key, "nash-equilibrium-finder");
        }
    }

    #[test]
    fn test_unknown_agent_gets_no_assignments() {
        let outputs = vec![SpecialistOutput {
            agent_key: "nonexistent-agent".to_string(),
            content: "Nothing.".to_string(),
            quality_checks: vec![],
        }];

        let assignments = map_to_sections(&outputs);
        assert!(assignments.is_empty(), "unknown agent has no section mapping");
    }
}
