//! Game-theory orchestration entrypoint.
//!
//! The `classify` function takes a situation string, builds the Tier 1 spec,
//! generates a classification fingerprint, and persists everything to Cozo.

use anyhow::Result;
use cozo::DbInstance;
use chrono::Utc;

use super::errors::GameTheoryError;
use super::fingerprint::{
    AmbiguityNote, AxisVerdict, GameTheoryFingerprint, HiddenGameDetection,
};
use super::schema::ensure_gametheory_schema;

/// Run Tier 1 classification on a situation and persist the fingerprint.
///
/// Returns the generated fingerprint after persistence.
pub fn classify(db: &DbInstance, situation: &str) -> Result<GameTheoryFingerprint, GameTheoryError> {
    let situation = situation.trim();
    if situation.is_empty() {
        return Err(GameTheoryError::EmptySituation);
    }

    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    let run_id = format!("gt-{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string());
    let now = Utc::now().to_rfc3339();

    // Insert run with status "running"
    insert_gt_run(db, &run_id, situation, &now, "running")
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;

    // Generate synthetic fingerprint (placeholder for real Tier 1 agent execution)
    let fingerprint = generate_synthetic_fingerprint(&run_id, situation, &now);

    // Persist fingerprint
    let fingerprint_json =
        serde_json::to_string(&fingerprint).map_err(|e| GameTheoryError::FingerprintParse {
            message: e.to_string(),
        })?;

    insert_gt_fingerprint(db, &run_id, &fingerprint_json, &fingerprint.primary_family, &now)
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;

    // Update run status to completed
    let completed_at = Utc::now().to_rfc3339();
    update_gt_run_status(db, &run_id, situation, &now, &completed_at, "completed")
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;

    Ok(fingerprint)
}

/// Generate a synthetic fingerprint from keyword analysis of the situation.
///
/// This is a placeholder for real Tier 1 agent execution. In Phase 4, this
/// will be replaced by actual LLM agent outputs parsed from the DAG results.
fn generate_synthetic_fingerprint(
    run_id: &str,
    situation: &str,
    now: &str,
) -> GameTheoryFingerprint {
    let s = situation.to_lowercase();

    let cooperation = if s.contains("collaborate") || s.contains("cooperate") || s.contains("alliance") || s.contains("cartel") {
        AxisVerdict::new("cooperative", "medium", "cooperation keywords detected")
    } else {
        AxisVerdict::new("non-cooperative", "medium", "default for unmarked situations")
    };

    let payoff_sum = if s.contains("zero-sum") || s.contains("winner-take") || s.contains("all or nothing") {
        AxisVerdict::new("zero-sum", "medium", "zero-sum keywords detected")
    } else if s.contains("win-win") || s.contains("mutual gain") || s.contains("positive-sum") {
        AxisVerdict::new("positive-sum", "medium", "positive-sum keywords detected")
    } else {
        AxisVerdict::new("variable-sum", "low", "insufficient payoff information")
    };

    let symmetry = if s.contains("symmetric") || s.contains("identical") || s.contains("same") {
        AxisVerdict::new("symmetric", "medium", "symmetry keywords detected")
    } else if s.contains("asymmetric") || s.contains("different") {
        AxisVerdict::new("asymmetric", "medium", "asymmetry keywords detected")
    } else {
        AxisVerdict::new("unknown", "low", "insufficient symmetry information")
    };

    let timing = if s.contains("simultaneous") || s.contains("at the same time") {
        AxisVerdict::new("simultaneous", "medium", "simultaneous keyword detected")
    } else if s.contains("sequential") || s.contains("take turns") || s.contains("first mover") {
        AxisVerdict::new("sequential", "medium", "sequential keyword detected")
    } else if s.contains("repeated") || s.contains("ongoing") {
        AxisVerdict::new("repeated", "medium", "repeated keyword detected")
    } else {
        AxisVerdict::new("simultaneous", "low", "default assumption")
    };

    let perfect_info = if s.contains("perfect information") || s.contains("knows everything") || s.contains("full information") {
        AxisVerdict::new("perfect", "medium", "perfect information keywords")
    } else if s.contains("imperfect") || s.contains("hidden") || s.contains("private") {
        AxisVerdict::new("imperfect", "medium", "imperfect information keywords")
    } else {
        AxisVerdict::new("imperfect", "low", "most real situations have imperfect info")
    };

    let complete_info = if s.contains("incomplete") || s.contains("doesn't know") || s.contains("unknown") || s.contains("private type") || s.contains("asymmetric information") {
        AxisVerdict::new("incomplete", "medium", "incomplete information keywords")
    } else if s.contains("complete information") || s.contains("knows everything about") {
        AxisVerdict::new("complete", "medium", "complete information keywords")
    } else {
        AxisVerdict::new("incomplete", "low", "most real situations have incomplete info")
    };

    let cardinality = if s.contains("two player") || s.contains("two firm") || s.contains("bilateral") || s.contains("duopoly") || (s.contains("two") && s.contains("player")) {
        AxisVerdict::new("2-player", "medium", "two-player keywords")
    } else if s.contains("n-player") || s.contains("multi") || s.contains("many") || s.contains("oligopoly") || s.contains("market") {
        AxisVerdict::new("n-player", "medium", "multi-player keywords")
    } else {
        AxisVerdict::new("2-player", "low", "default assumption")
    };

    let strategy_space = if s.contains("continuous") || s.contains("price") || s.contains("quantity") || s.contains("amount") {
        AxisVerdict::new("continuous", "medium", "continuous strategy indicators")
    } else if s.contains("discrete") || s.contains("binary") || s.contains("yes/no") || s.contains("choice") {
        AxisVerdict::new("discrete", "medium", "discrete strategy indicators")
    } else {
        AxisVerdict::new("discrete", "low", "default assumption")
    };

    let horizon = if s.contains("one-shot") || s.contains("once") || s.contains("single") {
        AxisVerdict::new("one-shot", "medium", "one-shot keywords")
    } else if s.contains("repeated") || s.contains("ongoing") || s.contains("infinitely") || s.contains("recurrent") {
        AxisVerdict::new("repeated", "medium", "repeated keywords")
    } else {
        AxisVerdict::new("one-shot", "low", "default assumption")
    };

    let (primary_family, nearest_classic) = if s.contains("price") && s.contains("simultaneous") {
        ("Bertrand competition".into(), Some("Bertrand duopoly".into()))
    } else if s.contains("quantity") && s.contains("simultaneous") {
        ("Cournot competition".into(), Some("Cournot duopoly".into()))
    } else if s.contains("price") && s.contains("sequential") {
        ("Stackelberg price leadership".into(), Some("Stackelberg duopoly".into()))
    } else if s.contains("dilemma") || s.contains("defect") || s.contains("cooperate vs") {
        ("Social dilemma".into(), Some("Prisoner's Dilemma".into()))
    } else if s.contains("coordinate") || s.contains("standard") || s.contains("compatible") {
        ("Coordination game".into(), Some("Battle of the Sexes".into()))
    } else if s.contains("auction") || s.contains("bid") {
        ("Auction".into(), Some("First-price sealed-bid auction".into()))
    } else if s.contains("negotiate") || s.contains("bargain") || s.contains("offer") {
        ("Bargaining".into(), Some("Ultimatum Game".into()))
    } else if s.contains("deter") || s.contains("threat") || s.contains("retaliate") {
        ("Deterrence".into(), Some("Chicken / Hawk-Dove".into()))
    } else {
        ("Strategic interaction".into(), None::<String>)
    };

    let ambiguities = if situation.len() < 50 {
        vec![AmbiguityNote {
            axis: "all".into(),
            note: "situation too brief for confident classification".into(),
        }]
    } else if !s.contains("payoff") && !s.contains("utility") && !s.contains("profit") && !s.contains("cost") {
        vec![AmbiguityNote {
            axis: "payoff_sum".into(),
            note: "no payoff or utility information provided".into(),
        }]
    } else {
        vec![]
    };

    let shadow_games: Vec<String> = if s.contains("price") && !s.contains("collude") && !s.contains("cartel") {
        vec!["Prisoner's Dilemma (tacit collusion shadow)".into()]
    } else {
        vec![]
    };

    let hidden_game_scan = if !shadow_games.is_empty() {
        Some(HiddenGameDetection {
            game_name: shadow_games[0].clone(),
            confidence: "low".into(),
            description: "potential hidden cooperative structure in competitive framing".into(),
        })
    } else {
        None
    };

    GameTheoryFingerprint {
        run_id: run_id.to_string(),
        cooperation,
        payoff_sum,
        symmetry,
        timing,
        perfect_info,
        complete_info,
        cardinality,
        strategy_space,
        horizon,
        primary_family,
        nearest_classic,
        shadow_games,
        hidden_game_scan,
        ambiguities,
        created_at: now.to_string(),
    }
}

// Cozo helpers

fn insert_gt_run(
    db: &DbInstance,
    run_id: &str,
    situation: &str,
    started_at: &str,
    status: &str,
) -> Result<()> {
    use std::collections::BTreeMap;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("sit".into(), cozo::DataValue::from(situation));
    params.insert("sa".into(), cozo::DataValue::from(started_at));
    params.insert("st".into(), cozo::DataValue::from(status));

    db.run_script(
        "?[run_id, situation, started_at, completed_at, status] \
         <- [[$rid, $sit, $sa, \"\", $st]] \
         :put gt_runs { run_id => situation, started_at, completed_at, status }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert gt_runs failed: {e}"))?;
    Ok(())
}

fn update_gt_run_status(
    db: &DbInstance,
    run_id: &str,
    situation: &str,
    started_at: &str,
    completed_at: &str,
    status: &str,
) -> Result<()> {
    use std::collections::BTreeMap;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("sit".into(), cozo::DataValue::from(situation));
    params.insert("sa".into(), cozo::DataValue::from(started_at));
    params.insert("ca".into(), cozo::DataValue::from(completed_at));
    params.insert("st".into(), cozo::DataValue::from(status));

    db.run_script(
        "?[run_id, situation, started_at, completed_at, status] \
         <- [[$rid, $sit, $sa, $ca, $st]] \
         :put gt_runs { run_id => situation, started_at, completed_at, status }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("update gt_runs failed: {e}"))?;
    Ok(())
}

fn insert_gt_fingerprint(
    db: &DbInstance,
    run_id: &str,
    fingerprint_json: &str,
    primary_family: &str,
    created_at: &str,
) -> Result<()> {
    use std::collections::BTreeMap;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("fp".into(), cozo::DataValue::from(fingerprint_json));
    params.insert("pf".into(), cozo::DataValue::from(primary_family));
    params.insert("ca".into(), cozo::DataValue::from(created_at));

    db.run_script(
        "?[run_id, fingerprint_json, primary_family, created_at] \
         <- [[$rid, $fp, $pf, $ca]] \
         :put gt_fingerprints { run_id => fingerprint_json, primary_family, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert gt_fingerprints failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-gt-facade-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn test_empty_situation_rejected() {
        let db = test_db();
        let err = classify(&db, "").unwrap_err();
        assert!(matches!(err, GameTheoryError::EmptySituation));
    }

    #[test]
    fn test_classify_only_persists_run_and_fingerprint() {
        let db = test_db();
        let fp = classify(&db, "Two firms simultaneously set prices.").unwrap();

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
        let fp = classify(
            &db,
            "Two firms simultaneously set prices, neither knows the other's cost.",
        )
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
}
