//! Section types and specialist-to-section mapping for final report assembly.
//!
//! Each section type maps to a title and a set of specialist agent keys whose
//! outputs feed into that section. The mapping table is the single source of
//! truth for section ordering and specialist assignment.

use std::collections::BTreeMap;

use super::registry::GAMETHEORY_AGENTS;

/// The standard sections in a strategic game-theory report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum SectionType {
    ExecutiveSummary,
    SituationAndAssumptions,
    NineAxisFingerprint,
    PayoffAndStrategyStructure,
    EquilibriumAnalysis,
    InformationSignallingAnalysis,
    BehaviouralEvolutionaryDynamics,
    MechanismDesignInterventionOptions,
    Recommendations,
    RisksUnknownsContradictions,
    ProvenanceFooter,
}

impl SectionType {
    /// Human-readable title for each section.
    pub fn title(&self) -> &'static str {
        match self {
            Self::ExecutiveSummary => "Executive Summary",
            Self::SituationAndAssumptions => "Situation and Assumptions",
            Self::NineAxisFingerprint => "9-Axis Fingerprint",
            Self::PayoffAndStrategyStructure => "Payoff and Strategy Structure",
            Self::EquilibriumAnalysis => "Equilibrium Analysis",
            Self::InformationSignallingAnalysis => "Information and Signalling Analysis",
            Self::BehaviouralEvolutionaryDynamics => "Behavioural and Evolutionary Dynamics",
            Self::MechanismDesignInterventionOptions => "Mechanism Design / Intervention Options",
            Self::Recommendations => "Recommendations",
            Self::RisksUnknownsContradictions => "Risks, Unknowns and Contradictions",
            Self::ProvenanceFooter => "Provenance Footer",
        }
    }

    /// Ordered precedence for report assembly.
    pub fn order(&self) -> u8 {
        match self {
            Self::ExecutiveSummary => 0,
            Self::SituationAndAssumptions => 1,
            Self::NineAxisFingerprint => 2,
            Self::PayoffAndStrategyStructure => 3,
            Self::EquilibriumAnalysis => 4,
            Self::InformationSignallingAnalysis => 5,
            Self::BehaviouralEvolutionaryDynamics => 6,
            Self::MechanismDesignInterventionOptions => 7,
            Self::Recommendations => 8,
            Self::RisksUnknownsContradictions => 9,
            Self::ProvenanceFooter => 10,
        }
    }

    /// All section types in display order.
    pub fn all_ordered() -> Vec<SectionType> {
        vec![
            Self::ExecutiveSummary,
            Self::SituationAndAssumptions,
            Self::NineAxisFingerprint,
            Self::PayoffAndStrategyStructure,
            Self::EquilibriumAnalysis,
            Self::InformationSignallingAnalysis,
            Self::BehaviouralEvolutionaryDynamics,
            Self::MechanismDesignInterventionOptions,
            Self::Recommendations,
            Self::RisksUnknownsContradictions,
            Self::ProvenanceFooter,
        ]
    }
}

/// Maps specialist agent keys to the sections they contribute to.
///
/// A specialist may contribute to multiple sections (e.g. a Nash equilibrium
/// finder feeds both EquilibriumAnalysis and FormalAnalysis).
pub fn specialist_section_map() -> BTreeMap<&'static str, Vec<SectionType>> {
    let mut map = BTreeMap::new();

    for agent in GAMETHEORY_AGENTS {
        map.insert(agent.key, sections_for_tier_agent(agent.tier, agent.key));
    }

    map
}

/// Given a specialist agent key, return the sections it contributes to.
pub fn sections_for_specialist(agent_key: &str) -> Vec<SectionType> {
    specialist_section_map()
        .get(agent_key)
        .cloned()
        .unwrap_or_default()
}

fn sections_for_tier_agent(tier: u8, agent_key: &str) -> Vec<SectionType> {
    use SectionType::*;

    match agent_key {
        "game-classifier" => vec![ExecutiveSummary, NineAxisFingerprint],
        "payoff-elicitor" | "payoff-matrix-builder" | "strategy-space-enumerator" => {
            vec![PayoffAndStrategyStructure]
        }
        "information-structure-mapper" => vec![InformationSignallingAnalysis],
        "nash-equilibrium-finder"
        | "dominant-strategy-identifier"
        | "mixed-strategy-calculator"
        | "bayesian-equilibrium-analyst"
        | "correlated-equilibrium-designer"
        | "subgame-perfect-analyzer"
        | "trembling-hand-refiner" => vec![EquilibriumAnalysis],
        "mechanism-designer"
        | "screening-mechanism-designer"
        | "auction-strategist"
        | "vcg-architect"
        | "incentive-compatibility-auditor"
        | "matching-market-designer"
        | "revenue-equivalence-analyst" => vec![MechanismDesignInterventionOptions],
        _ => match tier {
            1 => vec![SituationAndAssumptions],
            2 => vec![EquilibriumAnalysis],
            3 => vec![PayoffAndStrategyStructure],
            4 => vec![RisksUnknownsContradictions],
            5 => vec![EquilibriumAnalysis, BehaviouralEvolutionaryDynamics],
            6 => vec![InformationSignallingAnalysis],
            7 => vec![MechanismDesignInterventionOptions],
            8 => vec![BehaviouralEvolutionaryDynamics],
            9 => vec![Recommendations],
            10 => vec![Recommendations],
            11 => vec![RisksUnknownsContradictions],
            12 => vec![RisksUnknownsContradictions],
            _ => vec![RisksUnknownsContradictions],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_ordering() {
        let ordered = SectionType::all_ordered();
        assert_eq!(ordered[0], SectionType::ExecutiveSummary, "exec summary first");
        assert_eq!(
            ordered.last(),
            Some(&SectionType::ProvenanceFooter),
            "provenance footer last"
        );
        // Verify ascending order values
        for i in 1..ordered.len() {
            assert!(
                ordered[i - 1].order() < ordered[i].order(),
                "sections must be in ascending order"
            );
        }
    }

    #[test]
    fn test_sections_count_is_11() {
        assert_eq!(
            SectionType::all_ordered().len(),
            11,
            "PRD section schema must have 11 sections"
        );
    }

    #[test]
    fn test_specialist_section_map_has_entries() {
        let map = specialist_section_map();
        assert!(!map.is_empty(), "mapping table must be non-empty");
        // Tier 1 mandatory agents must have entries
        assert!(map.contains_key("game-classifier"));
        assert!(map.contains_key("payoff-matrix-builder"));
    }

    #[test]
    fn test_every_specialist_maps_to_at_least_one_section() {
        let map = specialist_section_map();
        for agent in GAMETHEORY_AGENTS {
            let sections = map.get(agent.key).expect("agent must be in section map");
            assert!(!sections.is_empty(), "{} must map to a report section", agent.key);
        }
        assert_eq!(map.len(), GAMETHEORY_AGENTS.len());
    }
}
