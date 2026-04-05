//! Default behavioral rules loaded on first run.

use std::collections::HashSet;

use crate::rules::{RuleSource, RulesEngine, RulesError};

/// Built-in fallback rules used when no `initial_rules` are configured.
const DEFAULTS: &[&str] = &[
    "Always ask before modifying files",
    "Explain reasoning before acting",
    "Never create files unless explicitly requested",
];

/// Idempotent rule seeding from a config-supplied list.
///
/// Behaviour:
/// - Builds the set of existing rule texts (trimmed).
/// - Determines the candidate list: `initial_rules` if non-empty, else `DEFAULTS`.
/// - For each candidate not already present, adds it with `RuleSource::SystemDefault`.
/// - Returns the count of rules actually added (0 when fully idempotent).
///
/// Calling this twice with the same inputs never produces duplicates.
/// Adding a new rule to `initial_rules` on an existing install inserts
/// only that new rule on the next startup.
pub fn load_configured_defaults(
    engine: &RulesEngine<'_>,
    initial_rules: &[String],
) -> Result<usize, RulesError> {
    let existing: HashSet<String> = engine
        .get_rules_sorted()?
        .into_iter()
        .map(|r| r.text.trim().to_string())
        .collect();

    let candidates: Vec<&str> = if initial_rules.is_empty() {
        // Fall back to hardcoded defaults only when engine is also empty.
        if !existing.is_empty() {
            return Ok(0);
        }
        DEFAULTS.to_vec()
    } else {
        initial_rules.iter().map(|s| s.as_str()).collect()
    };

    let mut added = 0;
    for text in candidates {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if existing.contains(trimmed) {
            continue;
        }
        engine.add_rule(trimmed, RuleSource::SystemDefault)?;
        added += 1;
    }

    Ok(added)
}

/// Backwards-compatible wrapper — delegates to [`load_configured_defaults`]
/// with an empty `initial_rules` slice (uses built-in defaults when engine
/// is empty, no-ops otherwise).
pub fn load_defaults(engine: &RulesEngine<'_>) -> Result<usize, RulesError> {
    load_configured_defaults(engine, &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::RulesEngine;
    use archon_memory::MemoryGraph;

    // ── load_configured_defaults tests ────────────────────────────────────

    #[test]
    fn configured_defaults_with_custom_rules() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        let rules_cfg = vec!["rule X".to_string(), "rule Y".to_string()];
        let count = load_configured_defaults(&engine, &rules_cfg).expect("load");
        assert_eq!(count, 2);

        let rules = engine.get_rules_sorted().expect("list");
        let texts: Vec<&str> = rules.iter().map(|r| r.text.as_str()).collect();
        assert!(texts.contains(&"rule X"));
        assert!(texts.contains(&"rule Y"));
    }

    #[test]
    fn configured_defaults_idempotent() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        let rules_cfg = vec!["rule A".to_string()];
        load_configured_defaults(&engine, &rules_cfg).expect("first");
        let count = load_configured_defaults(&engine, &rules_cfg).expect("second");
        assert_eq!(count, 0, "second call should add 0 rules");

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn configured_defaults_adds_new_rules_only() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        // Pre-seed "rule A"
        let first = vec!["rule A".to_string()];
        load_configured_defaults(&engine, &first).expect("seed A");

        // Now add both A and B — only B should be added
        let both = vec!["rule A".to_string(), "rule B".to_string()];
        let count = load_configured_defaults(&engine, &both).expect("seed A+B");
        assert_eq!(count, 1);

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn configured_defaults_skips_empty_rules() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        let rules_cfg = vec!["".to_string(), "  ".to_string(), "valid rule".to_string()];
        let count = load_configured_defaults(&engine, &rules_cfg).expect("load");
        assert_eq!(count, 1);

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].text, "valid rule");
    }

    #[test]
    fn configured_defaults_fallback_to_hardcoded() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        let count = load_configured_defaults(&engine, &[]).expect("load");
        assert_eq!(count, 3, "should seed 3 built-in defaults");
    }

    #[test]
    fn configured_defaults_no_seed_when_rules_exist_and_empty_config() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        engine
            .add_rule("existing", crate::rules::RuleSource::UserDefined)
            .expect("add");

        let count = load_configured_defaults(&engine, &[]).expect("load");
        assert_eq!(
            count, 0,
            "should not add defaults when rules exist and config is empty"
        );
    }

    #[test]
    fn backward_compat_load_defaults() {
        let graph = MemoryGraph::in_memory().expect("graph");
        let engine = RulesEngine::new(&graph);

        let count = load_defaults(&engine).expect("load");
        assert_eq!(count, 3);
    }

    // ── original tests ────────────────────────────────────────────────────

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
