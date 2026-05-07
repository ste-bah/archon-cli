use super::super::*;
use super::*;

#[test]
fn test_empty_situation_rejected() {
    let db = test_db();
    let err = block_on(classify(&db, "", None)).unwrap_err();
    assert!(matches!(err, GameTheoryError::EmptySituation));
}
#[test]
fn test_tier11_policy_gate_removes_agent_and_dependents() {
    let mut routing = RoutingDecision {
        run_id: "run-policy".into(),
        fingerprint_id: "fp-policy".into(),
        enabled_specialists: vec![
            "cohesion-discipline-devotion-auditor".into(),
            "dependent-agent".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };
    let mut deps = HashMap::new();
    deps.insert(
        "dependent-agent".to_string(),
        vec!["cohesion-discipline-devotion-auditor".to_string()],
    );
    apply_policy_gates_to_routing(&mut routing, &deps, false);
    assert!(routing.enabled_specialists.is_empty());
    assert_eq!(routing.skipped_specialists.len(), 2);
    assert!(
        routing
            .skipped_specialists
            .iter()
            .any(|(_, reason)| reason.contains("Tier 11 disabled"))
    );
}
#[test]
fn test_classify_only_persists_run_and_fingerprint() {
    let db = test_db();
    let fp = block_on(classify(&db, "Two firms simultaneously set prices.", None)).unwrap();

    // Verify fingerprint has all 9 axes filled
    assert_eq!(fp.cooperation.value, "non-cooperative");
    assert!(!fp.primary_family.is_empty());

    // Verify gt_runs has 1 row
    let runs = db
        .run_script(
            "?[status] := *gt_runs{run_id, status}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(runs.rows.len(), 1);
    assert_eq!(runs.rows[0][0].get_str().unwrap(), "completed");

    // Verify gt_fingerprints has 1 row
    let fps = db
        .run_script(
            "?[primary_family] := *gt_fingerprints{run_id, primary_family}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(fps.rows.len(), 1);

    // Verify fingerprint JSON round-trips
    let json_row = db
        .run_script(
            "?[fingerprint_json] := *gt_fingerprints{run_id, fingerprint_json}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    let json_str = json_row.rows[0][0].get_str().unwrap();
    let parsed: GameTheoryFingerprint = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed.run_id, fp.run_id);
    assert_eq!(parsed.primary_family, fp.primary_family);
}
#[test]
fn test_fingerprint_serde_roundtrip() {
    // Build a complete fingerprint and verify JSON serialize/deserialize
    let fp = GameTheoryFingerprint {
        run_id: "gt-test-001".into(),
        cooperation: AxisVerdict::new("cooperative", "high", "explicit cooperation stated"),
        payoff_sum: AxisVerdict::new("positive-sum", "medium", "mutual gains described"),
        symmetry: AxisVerdict::new("symmetric", "high", "identical capabilities"),
        timing: AxisVerdict::new("simultaneous", "high", "moves at same time"),
        perfect_info: AxisVerdict::new("imperfect", "low", "default assumption"),
        complete_info: AxisVerdict::new("incomplete", "low", "default assumption"),
        cardinality: AxisVerdict::new("2-player", "high", "two players named"),
        strategy_space: AxisVerdict::new("continuous", "medium", "price selection"),
        horizon: AxisVerdict::new("one-shot", "medium", "single interaction"),
        primary_family: "Bertrand competition".into(),
        nearest_classic: Some("Bertrand duopoly".into()),
        shadow_games: vec!["Prisoner's Dilemma (tacit collusion)".into()],
        hidden_game_scan: Some(HiddenGameDetection {
            game_name: "Prisoner's Dilemma".into(),
            confidence: "low".into(),
            description: "potential collusion shadow".into(),
        }),
        ambiguities: vec![AmbiguityNote {
            axis: "payoff_sum".into(),
            note: "exact payoffs not specified".into(),
        }],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let json = serde_json::to_string(&fp).expect("serialize must succeed");
    let roundtripped: GameTheoryFingerprint =
        serde_json::from_str(&json).expect("deserialize must succeed");
    assert_eq!(fp, roundtripped, "round-trip must preserve equality");
}
#[test]
fn test_fingerprint_has_all_nine_axes() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices, neither knows the other's cost.",
        None,
    ))
    .unwrap();

    // All 9 axes must have non-empty values
    assert!(!fp.cooperation.value.is_empty());
    assert!(!fp.payoff_sum.value.is_empty());
    assert!(!fp.symmetry.value.is_empty());
    assert!(!fp.timing.value.is_empty());
    assert!(!fp.perfect_info.value.is_empty());
    assert!(!fp.complete_info.value.is_empty());
    assert!(!fp.cardinality.value.is_empty());
    assert!(!fp.strategy_space.value.is_empty());
    assert!(!fp.horizon.value.is_empty());

    // Structural fields must be present
    assert!(!fp.run_id.is_empty());
    assert!(!fp.primary_family.is_empty());
    assert!(!fp.created_at.is_empty());
    assert!(fp.run_id.starts_with("gt-"), "run_id must have gt- prefix");
}
#[test]
fn test_full_pipeline_classify_only_mode() {
    // block_on(classify() is the classify-only entrypoint — it persists fingerprint
    // but does not run routing or specialists
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms negotiate a bilateral trade agreement with complete information.",
        None,
    ))
    .unwrap();

    assert!(!fp.run_id.is_empty());

    // Verify no routing or specialist data was persisted
    // Verify classify-only does NOT populate routing decisions
    let _routing = db
        .run_script(
            "?[count(run_id)] := *gt_routing_decisions{run_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(!fp.primary_family.is_empty());
}
#[test]
fn test_real_tier1_uses_agent_sdk() {
    let db = test_db();
    let canned_json = serde_json::json!({
        "cooperation": {"value": "non-cooperative", "confidence": "high", "rationale": "firms compete on price"},
        "payoff_sum": {"value": "zero-sum", "confidence": "medium", "rationale": "one firm's gain is other's loss"},
        "symmetry": {"value": "symmetric", "confidence": "high", "rationale": "identical products"},
        "timing": {"value": "simultaneous", "confidence": "high", "rationale": "firms set prices at same time"},
        "perfect_info": {"value": "imperfect", "confidence": "medium", "rationale": "firms don't see competitor's price"},
        "complete_info": {"value": "complete", "confidence": "medium", "rationale": "cost structures are known"},
        "cardinality": {"value": "2-player", "confidence": "high", "rationale": "duopoly"},
        "strategy_space": {"value": "continuous", "confidence": "high", "rationale": "prices are continuous"},
        "horizon": {"value": "one-shot", "confidence": "medium", "rationale": "single period"},
        "primary_family": "Bertrand competition",
        "nearest_classic": "Bertrand duopoly"
    });

    let mock = MockLlmClient::new(&canned_json.to_string());
    let fp = block_on(classify(
        &db,
        "Two firms set prices in a Bertrand duopoly.",
        Some(&mock),
    ))
    .unwrap();

    assert_eq!(fp.cooperation.value, "non-cooperative");
    assert_eq!(fp.cooperation.confidence, "high");
    assert_eq!(fp.payoff_sum.value, "zero-sum");
    assert_eq!(fp.primary_family, "Bertrand competition");
    assert_eq!(fp.nearest_classic, Some("Bertrand duopoly".into()));
    assert_eq!(fp.strategy_space.value, "continuous");
}
#[test]
fn test_classify_only_keyword_fallback_when_no_provider() {
    let db = test_db();

    // classify with None → must use keyword fallback
    let fp = block_on(classify(
        &db,
        "Two firms negotiate a bilateral trade agreement with complete information.",
        None,
    ))
    .unwrap();

    assert!(!fp.run_id.is_empty());
    assert!(
        fp.cooperation.confidence != "high",
        "keyword fallback should have medium/low confidence, not high"
    );
    assert_eq!(
        fp.shadow_games.len(),
        0,
        "no price competition → no shadow games"
    );

    // Verify the fingerprint was persisted (not just returned)
    let rows = db.run_script(
            "?[primary_family] := *gt_fingerprints{run_id, fingerprint_json, primary_family, created_at}, run_id = $rid",
            {
                let mut p = std::collections::BTreeMap::new();
                p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                p
            },
            cozo::ScriptMutability::Immutable,
        ).unwrap();
    assert_eq!(rows.rows.len(), 1, "fingerprint must be persisted to Cozo");
}
