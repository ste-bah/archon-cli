//! Default behavioral rules loaded on first run.

use crate::rules::{RuleSource, RulesEngine, RulesError};

/// Default rules and their initial scores.
const DEFAULTS: &[(&str, f64)] = &[
    ("Always ask before modifying files", 50.0),
    ("Explain reasoning before acting", 50.0),
    ("Never create files unless explicitly requested", 50.0),
];

/// Load default rules into the engine **only** if no rules exist yet.
///
/// Returns the number of rules that were inserted (0 when defaults
/// already exist).
pub fn load_defaults(engine: &RulesEngine<'_>) -> Result<usize, RulesError> {
    let existing = engine.get_rules_sorted()?;
    if !existing.is_empty() {
        return Ok(0);
    }

    for &(text, _score) in DEFAULTS {
        // `add_rule` sets the initial score to 50.0 which matches our
        // defaults.  If we ever need varying initial scores we can add
        // an `add_rule_with_score` variant.
        engine.add_rule(text, RuleSource::SystemDefault)?;
    }

    Ok(DEFAULTS.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::MemoryGraph;
    use crate::rules::RulesEngine;

    #[test]
    fn loads_defaults_when_empty() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        let count = load_defaults(&engine).expect("load");
        assert_eq!(count, 3);

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn does_not_duplicate_on_second_call() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        load_defaults(&engine).expect("first load");
        let count = load_defaults(&engine).expect("second load");
        assert_eq!(count, 0);

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn skips_when_user_rules_exist() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        engine
            .add_rule("custom rule", crate::rules::RuleSource::UserDefined)
            .expect("add");

        let count = load_defaults(&engine).expect("load");
        assert_eq!(count, 0);

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 1);
    }
}
