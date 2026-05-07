use super::*;
use crate::gametheory::fingerprint::AxisVerdict;
use crate::gametheory::{GameTheoryError, GameTheoryFingerprint};

fn make_fingerprint(overrides: &[(&str, &str)]) -> GameTheoryFingerprint {
    let mut fp = GameTheoryFingerprint {
        run_id: "test-run-1".into(),
        cooperation: AxisVerdict::new("non-cooperative", "high", ""),
        payoff_sum: AxisVerdict::new("zero-sum", "medium", ""),
        symmetry: AxisVerdict::new("asymmetric", "medium", ""),
        timing: AxisVerdict::new("simultaneous", "high", ""),
        perfect_info: AxisVerdict::new("imperfect", "medium", ""),
        complete_info: AxisVerdict::new("incomplete", "medium", ""),
        cardinality: AxisVerdict::new("2-player", "high", ""),
        strategy_space: AxisVerdict::new("continuous", "high", ""),
        horizon: AxisVerdict::new("one-shot", "medium", ""),
        primary_family: "Cournot competition".into(),
        nearest_classic: Some("Cournot duopoly".into()),
        shadow_games: vec![],
        hidden_game_scan: None,
        ambiguities: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };
    for &(axis, value) in overrides {
        match axis {
            "cooperation" => fp.cooperation.value = value.into(),
            "payoff_sum" => fp.payoff_sum.value = value.into(),
            "symmetry" => fp.symmetry.value = value.into(),
            "timing" => fp.timing.value = value.into(),
            "perfect_info" => fp.perfect_info.value = value.into(),
            "complete_info" => fp.complete_info.value = value.into(),
            "cardinality" => fp.cardinality.value = value.into(),
            "strategy_space" => fp.strategy_space.value = value.into(),
            "horizon" => fp.horizon.value = value.into(),
            _ => {}
        }
    }
    fp
}

fn make_two_tier_spec(extra_agents: Vec<AgentEntry>) -> GameTheorySpec {
    GameTheorySpec {
        version: "1.0".into(),
        spec_id: "test-spec".into(),
        cost_cap_usd: 5.0,
        tiers: vec![
            TierEntry {
                id: 1,
                name: "Foundation".into(),
                concurrency_cap: 4,
                agents: vec![
                    AgentEntry {
                        key: "gt-payoff".into(),
                        condition: None,
                        mandatory: true,
                        depends_on: vec![],
                    },
                    AgentEntry {
                        key: "gt-classify".into(),
                        condition: None,
                        mandatory: true,
                        depends_on: vec![],
                    },
                ],
            },
            TierEntry {
                id: 2,
                name: "Specialists".into(),
                concurrency_cap: 3,
                agents: extra_agents,
            },
        ],
    }
}

// ── Test 1: Grammar parsing ─────────────────────────────────────────

#[test]
fn test_routing_evaluator_parses_grammar() {
    let fp = make_fingerprint(&[]);

    // Exercise all operators: &&, ||, !, ==, !=, <, >, <=, >=
    let spec = make_two_tier_spec(vec![
            AgentEntry {
                key: "gt-and-test".into(),
                condition: Some(
                    "fingerprint.cooperation == 'non-cooperative' && fingerprint.strategy_space == 'continuous'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-or-test".into(),
                condition: Some(
                    "fingerprint.cooperation == 'cooperative' || fingerprint.payoff_sum == 'zero-sum'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-not-test".into(),
                condition: Some(
                    "! (fingerprint.cooperation == 'cooperative')"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-neq-test".into(),
                condition: Some(
                    "fingerprint.cardinality != 'n-player'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-lt-test".into(),
                condition: Some(
                    "fingerprint.cardinality < 'n-player'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-gte-test".into(),
                condition: Some(
                    "fingerprint.cardinality >= '2-player'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-nash-count-test".into(),
                condition: Some("nash_count >= 0".into()),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-enabled-count-test".into(),
                condition: Some("enabled_count >= 2".into()),
                mandatory: false,
                depends_on: vec![],
            },
        ]);

    let decision = evaluate_routing(&spec, &fp, "test-run", "2026-01-01T00:00:00Z").unwrap();

    // All 8 test conditions should be evaluated
    assert_eq!(decision.evaluated_conditions.len(), 8);

    // && test: both true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-and-test".to_string())
    );
    // || test: payoff_sum is zero-sum → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-or-test".to_string())
    );
    // ! test: cooperation is non-cooperative, so !(cooperative) → true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-not-test".to_string())
    );
    // != test: 2-player != n-player → true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-neq-test".to_string())
    );
    // < test: "2-player" < "n-player" lexicographically → true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-lt-test".to_string())
    );
    // >= test: "2-player" >= "2-player" → true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-gte-test".to_string())
    );
    // nash_count >= 0 → true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-nash-count-test".to_string())
    );
    // enabled_count >= 2 (2 mandatory agents already counted) → true → enabled
    assert!(
        decision
            .enabled_specialists
            .contains(&"gt-enabled-count-test".to_string())
    );

    // 2 mandatory + 8 conditional = 10 enabled
    assert_eq!(decision.enabled_specialists.len(), 10);
}

// ── Test 2: Determinism ─────────────────────────────────────────────

#[test]
fn test_routing_deterministic_over_fixed_fingerprint() {
    let fp = make_fingerprint(&[]);
    let spec = make_two_tier_spec(vec![
        AgentEntry {
            key: "gt-cond-1".into(),
            condition: Some("fingerprint.cooperation == 'non-cooperative'".into()),
            mandatory: false,
            depends_on: vec![],
        },
        AgentEntry {
            key: "gt-cond-2".into(),
            condition: Some("fingerprint.payoff_sum == 'zero-sum'".into()),
            mandatory: false,
            depends_on: vec![],
        },
    ]);

    let first = evaluate_routing(&spec, &fp, "run-1", "2026-01-01T00:00:00Z").unwrap();
    for _ in 0..100 {
        let subsequent = evaluate_routing(&spec, &fp, "run-1", "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(first.enabled_specialists, subsequent.enabled_specialists);
        assert_eq!(first.skipped_specialists, subsequent.skipped_specialists);
    }
}

#[test]
fn test_routing_skips_agent_when_dependency_was_not_enabled() {
    let fp = make_fingerprint(&[]);
    let spec = make_two_tier_spec(vec![
        AgentEntry {
            key: "gt-disabled-dependency".into(),
            condition: Some("fingerprint.cooperation == 'cooperative'".into()),
            mandatory: false,
            depends_on: vec![],
        },
        AgentEntry {
            key: "gt-dependent".into(),
            condition: Some("fingerprint.payoff_sum == 'zero-sum'".into()),
            mandatory: false,
            depends_on: vec!["gt-disabled-dependency".into()],
        },
    ]);

    let decision = evaluate_routing(&spec, &fp, "run-deps", "2026-01-01T00:00:00Z").unwrap();

    assert!(
        !decision
            .enabled_specialists
            .contains(&"gt-dependent".to_string()),
        "dependent specialist must not run when its dependency was skipped"
    );
    assert!(
        decision.skipped_specialists.iter().any(|(key, reason)| {
            key == "gt-dependent" && reason.contains("gt-disabled-dependency")
        }),
        "skip reason must name the unmet dependency: {:?}",
        decision.skipped_specialists
    );
}

// ── Test 3: Invalid expression → ConditionError ─────────────────────

#[test]
fn test_routing_invalid_expression_returns_condition_error() {
    let fp = make_fingerprint(&[]);
    let spec = make_two_tier_spec(vec![AgentEntry {
        key: "gt-bad".into(),
        condition: Some("fingerprint.cooperation == 'non-cooperative' && &".into()),
        mandatory: false,
        depends_on: vec![],
    }]);

    let err = evaluate_routing(&spec, &fp, "run-bad", "2026-01-01T00:00:00Z").unwrap_err();
    match err {
        GameTheoryError::ConditionError {
            expression,
            message: _,
        } => {
            assert!(
                expression.contains("fingerprint.cooperation"),
                "error must include the offending expression"
            );
        }
        other => panic!("expected ConditionError, got {:?}", other),
    }
}

// ── Test 4: Cycle detection ─────────────────────────────────────────

#[test]
fn test_routing_cycle_detection() {
    let fp = make_fingerprint(&[]);

    // Agent A depends on B, B depends on A → cycle
    let spec = GameTheorySpec {
        version: "1.0".into(),
        spec_id: "cycle-test".into(),
        cost_cap_usd: 5.0,
        tiers: vec![
            TierEntry {
                id: 1,
                name: "Base".into(),
                concurrency_cap: 4,
                agents: vec![AgentEntry {
                    key: "gt-a".into(),
                    condition: Some("fingerprint.cooperation == 'non-cooperative'".into()),
                    mandatory: false,
                    depends_on: vec!["gt-b".into()],
                }],
            },
            TierEntry {
                id: 2,
                name: "Dependent".into(),
                concurrency_cap: 2,
                agents: vec![AgentEntry {
                    key: "gt-b".into(),
                    condition: Some("fingerprint.payoff_sum == 'zero-sum'".into()),
                    mandatory: false,
                    depends_on: vec!["gt-a".into()],
                }],
            },
        ],
    };

    let err = evaluate_routing(&spec, &fp, "cycle-run", "2026-01-01T00:00:00Z").unwrap_err();
    match err {
        GameTheoryError::RoutingCycle { cycle } => {
            assert!(cycle.contains("gt-a"), "cycle must mention involved agents");
            assert!(cycle.contains("gt-b"), "cycle must mention involved agents");
        }
        other => panic!("expected RoutingCycle, got {:?}", other),
    }
}

#[test]
fn test_resolve_spec_path_searches_upward() {
    // Create temp dir with nested structure: root/.archon/specs/gametheory.yaml
    let temp = std::env::temp_dir().join(format!("gt-spec-test-{}", uuid::Uuid::new_v4()));
    let spec_dir = temp.join(".archon").join("specs");
    std::fs::create_dir_all(&spec_dir).unwrap();
    let spec_file = spec_dir.join("gametheory.yaml");
    std::fs::write(
        &spec_file,
        "version: \"1.0\"\nspec_id: test\ncost_cap_usd: 1.0\ntiers: []\n",
    )
    .unwrap();

    // Create nested dirs: temp/a/b/c/ (3 levels deep)
    let deep = temp.join("a").join("b").join("c");
    std::fs::create_dir_all(&deep).unwrap();

    // Run resolver from deep dir with no explicit path
    let orig_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&deep).unwrap();

    // Clear env var to avoid interference
    unsafe { std::env::remove_var("ARCHON_SPEC_PATH") };

    let result = resolve_spec_path(None);
    std::env::set_current_dir(&orig_cwd).unwrap();

    assert!(
        result.is_ok(),
        "should find spec via upward walk: {:?}",
        result.err()
    );

    // Canonicalize BEFORE removing the temp dir — on macOS /tmp is
    // symlinked through /var → /private/var, so the unresolved
    // tempdir path and the looked-up path differ even though they
    // point at the same inode. We must canonicalize while the
    // files still exist (canonicalize() requires the path to
    // resolve), then assert, then clean up.
    let resolved = result.unwrap();
    let resolved_canonical = resolved
        .canonicalize()
        .expect("resolved spec path must canonicalize while temp dir exists");
    let expected_canonical = spec_file
        .canonicalize()
        .expect("expected spec path must canonicalize while temp dir exists");

    std::fs::remove_dir_all(&temp).unwrap();

    assert_eq!(resolved_canonical, expected_canonical);
}
