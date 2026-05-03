//! Section types and specialist-to-section mapping for final report assembly.
//!
//! Each section type maps to a title and a set of specialist agent keys whose
//! outputs feed into that section. The mapping table is the single source of
//! truth for section ordering and specialist assignment.

use std::collections::BTreeMap;

/// The standard sections in a strategic game-theory report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum SectionType {
    ExecutiveSummary,
    FormalAnalysis,
    ShadowGames,
    EquilibriumAnalysis,
    StrategicImplications,
    Recommendations,
}

impl SectionType {
    /// Human-readable title for each section.
    pub fn title(&self) -> &'static str {
        match self {
            Self::ExecutiveSummary => "Executive Summary",
            Self::FormalAnalysis => "Formal Game-Theoretic Analysis",
            Self::ShadowGames => "Shadow Games and Hidden Structures",
            Self::EquilibriumAnalysis => "Equilibrium Analysis",
            Self::StrategicImplications => "Strategic Implications",
            Self::Recommendations => "Recommendations",
        }
    }

    /// Ordered precedence for report assembly.
    pub fn order(&self) -> u8 {
        match self {
            Self::ExecutiveSummary => 0,
            Self::FormalAnalysis => 1,
            Self::ShadowGames => 2,
            Self::EquilibriumAnalysis => 3,
            Self::StrategicImplications => 4,
            Self::Recommendations => 5,
        }
    }

    /// All section types in display order.
    pub fn all_ordered() -> Vec<SectionType> {
        vec![
            Self::ExecutiveSummary,
            Self::FormalAnalysis,
            Self::ShadowGames,
            Self::EquilibriumAnalysis,
            Self::StrategicImplications,
            Self::Recommendations,
        ]
    }
}

/// Maps specialist agent keys to the sections they contribute to.
///
/// A specialist may contribute to multiple sections (e.g. a Nash equilibrium
/// finder feeds both EquilibriumAnalysis and FormalAnalysis).
pub fn specialist_section_map() -> BTreeMap<&'static str, Vec<SectionType>> {
    let mut map = BTreeMap::new();

    // Tier 1 classification agents feed ExecutiveSummary
    map.insert("game-tree-archaeologist", vec![SectionType::ExecutiveSummary]);
    map.insert("legitimacy-crisis-analyst", vec![SectionType::ExecutiveSummary]);
    map.insert("payoff-matrix-builder", vec![SectionType::ExecutiveSummary, SectionType::FormalAnalysis]);
    map.insert("subgame-perfect-analyzer", vec![SectionType::ExecutiveSummary, SectionType::FormalAnalysis]);

    // Equilibrium specialists
    map.insert("nash-equilibrium-finder", vec![SectionType::EquilibriumAnalysis, SectionType::FormalAnalysis]);
    map.insert("dominant-strategy-identifier", vec![SectionType::EquilibriumAnalysis]);
    map.insert("mixed-strategy-calculator", vec![SectionType::EquilibriumAnalysis]);
    map.insert("bayesian-equilibrium-analyst", vec![SectionType::EquilibriumAnalysis, SectionType::FormalAnalysis]);
    map.insert("trembling-hand-refiner", vec![SectionType::EquilibriumAnalysis]);
    map.insert("correlated-equilibrium-designer", vec![SectionType::EquilibriumAnalysis]);

    // Shadow game detection
    map.insert("shadow-game-detector", vec![SectionType::ShadowGames]);

    // Strategic recommendations
    map.insert("strategic-recommendations", vec![SectionType::Recommendations, SectionType::StrategicImplications]);

    map
}

/// Given a specialist agent key, return the sections it contributes to.
pub fn sections_for_specialist(agent_key: &str) -> Vec<SectionType> {
    specialist_section_map()
        .get(agent_key)
        .cloned()
        .unwrap_or_default()
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
            Some(&SectionType::Recommendations),
            "recommendations last"
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
    fn test_specialist_section_map_has_entries() {
        let map = specialist_section_map();
        assert!(!map.is_empty(), "mapping table must be non-empty");
        // Tier 1 mandatory agents must have entries
        assert!(map.contains_key("game-tree-archaeologist"));
        assert!(map.contains_key("payoff-matrix-builder"));
    }
}
