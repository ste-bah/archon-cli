use super::*;
use crate::gametheory::GameTheoryAgent;

#[test]
fn test_registry_count_is_84() {
    assert_eq!(
        GAMETHEORY_AGENTS.len(),
        84,
        "registry must contain exactly 84 agents (86 .md files minus sherlock-holmes + code-simplifier)"
    );
}

#[test]
fn test_registry_excludes_sherlock_and_simplifier() {
    for agent in GAMETHEORY_AGENTS.iter() {
        assert!(
            !agent.key.contains("sherlock"),
            "agent '{}' must not be sherlock-holmes",
            agent.key
        );
        assert!(
            !agent.key.contains("code-simplifier"),
            "agent '{}' must not be code-simplifier",
            agent.key
        );
    }
}

#[test]
fn test_tier1_has_four_mandatory() {
    let mandatory: Vec<&GameTheoryAgent> =
        GAMETHEORY_AGENTS.iter().filter(|a| a.mandatory).collect();
    assert_eq!(
        mandatory.len(),
        4,
        "Tier 1 must have exactly 4 mandatory agents"
    );
    let keys: Vec<&str> = mandatory.iter().map(|a| a.key).collect();
    assert!(keys.contains(&"game-classifier"));
    assert!(keys.contains(&"payoff-elicitor"));
    assert!(keys.contains(&"information-structure-mapper"));
    assert!(keys.contains(&"strategy-space-enumerator"));
}

#[test]
fn test_tiers_have_unique_ids() {
    let mut ids: Vec<u8> = GAMETHEORY_TIERS.iter().map(|t| t.id).collect();
    ids.sort();
    let orig_len = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), orig_len, "all tier IDs must be unique");
}

#[test]
fn test_no_docs2_paths_in_registry() {
    for agent in GAMETHEORY_AGENTS.iter() {
        assert!(
            !agent.prompt_source_path.contains("docs2/"),
            "agent '{}' has docs2/ in path: {}",
            agent.key,
            agent.prompt_source_path
        );
    }
}

#[test]
fn test_all_registry_paths_resolve_to_existing_files() {
    // CARGO_MANIFEST_DIR is crates/archon-pipeline/; workspace root is two levels up
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    for agent in GAMETHEORY_AGENTS.iter() {
        let full_path = workspace_root.join(agent.prompt_source_path);
        assert!(
            full_path.exists(),
            "agent '{}' path does not exist: {}",
            agent.key,
            full_path.display()
        );
    }
}

#[test]
fn test_registry_path_format_matches_other_categories() {
    // Verify gametheory paths use same ".archon/agents/<category>/" prefix as research agents
    let research_agent = &crate::research::agents::RESEARCH_AGENTS[0];
    let gt_agent = &GAMETHEORY_AGENTS[0];

    // Both paths start with ".archon/agents/"
    assert!(
        research_agent
            .prompt_source_path
            .starts_with(".archon/agents/"),
        "research agent path must use .archon/agents/ prefix"
    );
    assert!(
        gt_agent.prompt_source_path.starts_with(".archon/agents/"),
        "gametheory agent path must use .archon/agents/ prefix, got: {}",
        gt_agent.prompt_source_path
    );

    // Both follow the pattern .archon/agents/<category>/<name>.md
    let re_parts: Vec<&str> = research_agent.prompt_source_path.split('/').collect();
    let gt_parts: Vec<&str> = gt_agent.prompt_source_path.split('/').collect();
    assert_eq!(re_parts.len(), 4, "research path must have 4 segments");
    assert_eq!(
        gt_parts.len(),
        4,
        "gametheory path must have 4 segments, got: {}",
        gt_agent.prompt_source_path
    );
    assert!(
        gt_parts[3].ends_with(".md"),
        "last segment must be .md file"
    );
}

#[test]
fn test_yaml_tier1_matches_registry_mandatory_set() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let yaml_path = workspace_root.join(".archon/specs/gametheory.yaml");
    assert!(
        yaml_path.exists(),
        "gametheory.yaml not found at {}",
        yaml_path.display()
    );

    let spec =
        crate::gametheory::routing::load_spec(&yaml_path).expect("must load gametheory.yaml");

    // Collect Tier 1 mandatory agents from YAML
    let yaml_tier1: std::collections::HashSet<&str> = spec
        .tiers
        .iter()
        .filter(|t| t.id == 1)
        .flat_map(|t| t.agents.iter())
        .filter(|a| a.mandatory)
        .map(|a| a.key.as_str())
        .collect();

    // Collect mandatory agents from registry
    let registry_mandatory: std::collections::HashSet<&str> = GAMETHEORY_AGENTS
        .iter()
        .filter(|a| a.mandatory)
        .map(|a| a.key)
        .collect();

    assert_eq!(
        yaml_tier1, registry_mandatory,
        "YAML Tier 1 mandatory agents must exactly match registry mandatory agents.\n\
             YAML Tier 1 mandatory: {:?}\n\
             Registry mandatory:    {:?}",
        yaml_tier1, registry_mandatory
    );
}

#[test]
fn test_no_excluded_agent_files_present() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let sherlock_path = workspace_root.join(".archon/agents/gametheory/sherlock-holmes.md");
    let simplifier_path = workspace_root.join(".archon/agents/gametheory/code-simplifier.md");

    assert!(
        !sherlock_path.exists(),
        "sherlock-holmes.md must NOT exist at {} — it is a non-game-theory agent excluded per OQ-GT-001",
        sherlock_path.display()
    );
    assert!(
        !simplifier_path.exists(),
        "code-simplifier.md must NOT exist at {} — it is a non-game-theory agent excluded per OQ-GT-001",
        simplifier_path.display()
    );
}

// ── Group 2: Full 12-tier YAML coverage tests ────────────────────────────

fn load_yaml_spec() -> crate::gametheory::routing::GameTheorySpec {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let yaml_path = workspace_root.join(".archon/specs/gametheory.yaml");
    crate::gametheory::routing::load_spec(&yaml_path).expect("must load gametheory.yaml")
}

fn test_fingerprint(
    run_id: &str,
    cooperation: &str,
    payoff_sum: &str,
    timing: &str,
    cardinality: &str,
    horizon: &str,
) -> crate::gametheory::fingerprint::GameTheoryFingerprint {
    crate::gametheory::fingerprint::GameTheoryFingerprint {
        run_id: run_id.into(),
        cooperation: crate::gametheory::fingerprint::AxisVerdict::new(cooperation, "high", ""),
        payoff_sum: crate::gametheory::fingerprint::AxisVerdict::new(payoff_sum, "high", ""),
        symmetry: crate::gametheory::fingerprint::AxisVerdict::new("asymmetric", "medium", ""),
        timing: crate::gametheory::fingerprint::AxisVerdict::new(timing, "high", ""),
        perfect_info: crate::gametheory::fingerprint::AxisVerdict::new("imperfect", "medium", ""),
        complete_info: crate::gametheory::fingerprint::AxisVerdict::new("incomplete", "medium", ""),
        cardinality: crate::gametheory::fingerprint::AxisVerdict::new(cardinality, "high", ""),
        strategy_space: crate::gametheory::fingerprint::AxisVerdict::new("discrete", "medium", ""),
        horizon: crate::gametheory::fingerprint::AxisVerdict::new(horizon, "high", ""),
        primary_family: "test".into(),
        nearest_classic: None,
        shadow_games: vec![],
        hidden_game_scan: None,
        ambiguities: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    }
}

/// Every registry agent key must appear in the YAML spec.
#[test]
fn test_yaml_covers_all_84_agents() {
    let spec = load_yaml_spec();
    let yaml_keys: std::collections::HashSet<&str> = spec
        .tiers
        .iter()
        .flat_map(|t| t.agents.iter().map(|a| a.key.as_str()))
        .collect();

    let registry_keys: std::collections::HashSet<&str> =
        GAMETHEORY_AGENTS.iter().map(|a| a.key).collect();

    let missing: Vec<_> = registry_keys.difference(&yaml_keys).copied().collect();

    assert!(
        missing.is_empty(),
        "YAML spec is missing {} registry agent(s): {:?}",
        missing.len(),
        missing
    );
    assert_eq!(yaml_keys.len(), 84, "YAML must cover exactly 84 agents");
}

/// Every YAML agent key must have a matching registry entry.
#[test]
fn test_yaml_no_orphan_agents() {
    let spec = load_yaml_spec();
    let registry_keys: std::collections::HashSet<&str> =
        GAMETHEORY_AGENTS.iter().map(|a| a.key).collect();

    for tier in &spec.tiers {
        for agent in &tier.agents {
            assert!(
                registry_keys.contains(agent.key.as_str()),
                "YAML agent '{}' in tier {} has no matching registry entry",
                agent.key,
                tier.id
            );
        }
    }
}

/// The full spec depends_on graph must contain no cycles.
#[test]
fn test_yaml_no_cycles() {
    let spec = load_yaml_spec();
    let fp = crate::gametheory::fingerprint::GameTheoryFingerprint {
        run_id: "cycle-check".into(),
        cooperation: crate::gametheory::fingerprint::AxisVerdict::new(
            "non-cooperative",
            "high",
            "",
        ),
        payoff_sum: crate::gametheory::fingerprint::AxisVerdict::new("non-zero-sum", "high", ""),
        symmetry: crate::gametheory::fingerprint::AxisVerdict::new("asymmetric", "high", ""),
        timing: crate::gametheory::fingerprint::AxisVerdict::new("simultaneous", "high", ""),
        perfect_info: crate::gametheory::fingerprint::AxisVerdict::new("imperfect", "high", ""),
        complete_info: crate::gametheory::fingerprint::AxisVerdict::new("incomplete", "high", ""),
        cardinality: crate::gametheory::fingerprint::AxisVerdict::new("2-player", "high", ""),
        strategy_space: crate::gametheory::fingerprint::AxisVerdict::new("discrete", "high", ""),
        horizon: crate::gametheory::fingerprint::AxisVerdict::new("repeated", "high", ""),
        primary_family: "test".into(),
        nearest_classic: None,
        shadow_games: vec![],
        hidden_game_scan: None,
        ambiguities: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let result = crate::gametheory::routing::evaluate_routing(
        &spec,
        &fp,
        "cycle-check",
        "2026-05-03T00:00:00Z",
    );

    match result {
        Ok(decision) => {
            assert!(
                !decision.enabled_specialists.is_empty(),
                "routing must enable at least the 4 mandatory Tier 1 agents"
            );
        }
        Err(e) => panic!("YAML spec has errors (possibly a cycle): {e:?}"),
    }
}

#[test]
fn test_yaml_routing_never_enables_unmet_dependencies() {
    let spec = load_yaml_spec();
    let dep_map: std::collections::HashMap<&str, Vec<&str>> = spec
        .tiers
        .iter()
        .flat_map(|tier| {
            tier.agents.iter().map(|agent| {
                (
                    agent.key.as_str(),
                    agent
                        .depends_on
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                )
            })
        })
        .collect();
    let fingerprints = [
        test_fingerprint(
            "noncoop-repeated",
            "non-cooperative",
            "non-zero-sum",
            "simultaneous",
            "2-player",
            "repeated",
        ),
        test_fingerprint(
            "coop-nplayer",
            "cooperative",
            "non-zero-sum",
            "simultaneous",
            "n-player",
            "repeated",
        ),
        test_fingerprint(
            "sequential-zero-sum",
            "non-cooperative",
            "zero-sum",
            "sequential",
            "2-player",
            "one-shot",
        ),
    ];

    for fp in &fingerprints {
        let decision = crate::gametheory::routing::evaluate_routing(
            &spec,
            fp,
            &fp.run_id,
            "2026-05-03T00:00:00Z",
        )
        .expect("YAML routing must evaluate");
        let enabled: std::collections::HashSet<&str> = decision
            .enabled_specialists
            .iter()
            .map(String::as_str)
            .collect();

        for agent_key in &decision.enabled_specialists {
            let missing: Vec<&str> = dep_map
                .get(agent_key.as_str())
                .into_iter()
                .flat_map(|deps| deps.iter().copied())
                .filter(|dep| !enabled.contains(dep))
                .collect();
            assert!(
                missing.is_empty(),
                "run {} enabled '{}' with unmet dependencies {:?}",
                fp.run_id,
                agent_key,
                missing
            );
        }
    }
}

/// Every condition expression in the YAML must parse successfully.
#[test]
fn test_yaml_conditions_parse() {
    let spec = load_yaml_spec();
    let mut failed = Vec::new();

    for tier in &spec.tiers {
        for agent in &tier.agents {
            if let Some(ref cond) = agent.condition {
                if let Err(e) = crate::gametheory::routing::parse_condition(cond) {
                    failed.push((agent.key.clone(), cond.clone(), e));
                }
            }
        }
    }

    assert!(
        failed.is_empty(),
        "{} condition(s) failed to parse:\n{}",
        failed.len(),
        failed
            .iter()
            .map(|(key, cond, err)| format!("  {key}: '{cond}' → {err}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
