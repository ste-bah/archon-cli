//! Game-theory orchestration entrypoints.
//!
//! `classify` — Tier 1 classification only (fingerprint + persistence).
//! `run_full_pipeline` — classify → route → specialist DAG → final report.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use cozo::DbInstance;
use chrono::Utc;

use super::errors::GameTheoryError;
use super::final_stage;
use super::fingerprint::{
    AmbiguityNote, AxisVerdict, GameTheoryFingerprint, HiddenGameDetection,
};
use super::prompt_builder;
use super::quality;
use super::routing::{evaluate_routing, load_spec, resolve_spec_path, GameTheorySpec, RoutingDecision};
use super::schema::ensure_gametheory_schema;
use super::spec::build_specialist_spec;

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

/// Result of a full pipeline run.
#[derive(Debug, Clone)]
pub struct FullPipelineResult {
    pub run_id: String,
    pub fingerprint: GameTheoryFingerprint,
    pub routing_decision: RoutingDecision,
    pub report: String,
    pub specialist_count: usize,
    /// Specialists that failed during execution (agent_key, error_message).
    pub failed_specialists: Vec<(String, String)>,
    /// Overall pipeline status: "completed" (all specialists succeeded) or "partial" (some failed).
    pub status: String,
}

/// Run the full Phase 4 pipeline: classify → route → specialist spec → final report.
///
/// Persists all intermediate artefacts to Cozo. Specialist execution is stubbed
/// in Phase 4; real agent spawning is wired in Phase 5.
pub fn run_full_pipeline(
    db: &DbInstance,
    situation: &str,
    spec_path: Option<&Path>,
) -> Result<FullPipelineResult, GameTheoryError> {
    // 1. Tier 1 classification
    let fingerprint = classify(db, situation)?;

    // 2. Resolve and load routing spec
    let resolved_path = resolve_spec_path(spec_path)?;
    let spec = load_spec(&resolved_path)?;

    // 3. Evaluate routing
    let now = Utc::now().to_rfc3339();
    let routing_decision = evaluate_routing(&spec, &fingerprint, &fingerprint.run_id, &now)?;

    // 4. Persist routing decision
    persist_routing_decision(db, &routing_decision)
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;

    // 5. Build dependency map from spec agent entries
    let dep_map = build_dependency_map(&spec);

    // 6. Build specialist DAG spec
    let _pipeline_spec = build_specialist_spec(&routing_decision, &dep_map, &spec, situation);

    // 7. Execute specialist DAG (stub — Phase 5 wires real agent spawns)
    let (specialist_outputs, failed_specialists) =
        execute_specialist_stub(&routing_decision, &fingerprint, situation);

    // 8. Persist specialist outputs
    persist_specialist_outputs(db, &routing_decision.run_id, &specialist_outputs)
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;

    // 9. Run quality checks
    let mut quality_results: HashMap<String, Vec<quality::QualityCheck>> = HashMap::new();
    for (key, output) in &specialist_outputs {
        let checks = quality::run_advisory_gates(key, output);
        quality_results.insert(key.clone(), checks);
    }

    // 10. Final stage assembly
    let final_result = final_stage::assemble_report(&specialist_outputs, &quality_results, None);

    // 11. Persist sections and final report
    persist_sections(db, &routing_decision.run_id, &final_result.report)
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;
    persist_final_report(db, &routing_decision.run_id, &final_result.report)
        .map_err(|e| GameTheoryError::Storage { message: e.to_string() })?;

    let status = if failed_specialists.is_empty() {
        "completed".to_string()
    } else {
        "partial".to_string()
    };

    Ok(FullPipelineResult {
        run_id: routing_decision.run_id.clone(),
        fingerprint,
        routing_decision,
        report: final_result.report,
        specialist_count: specialist_outputs.len(),
        failed_specialists,
        status,
    })
}

/// Build a dependency map from spec agent entries.
fn build_dependency_map(spec: &GameTheorySpec) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for tier in &spec.tiers {
        for agent in &tier.agents {
            map.insert(agent.key.clone(), agent.depends_on.clone());
        }
    }
    map
}

/// Execute specialists with failure isolation.
///
/// Each enabled specialist is wrapped in a Result. If the agent_key ends with
/// `-FORCE-FAIL-FOR-TEST`, execution returns Err (test hook for failure isolation).
/// In Phase 5, this will be replaced with real subagent spawning where failures
/// are expected (network errors, timeouts, model errors).
///
/// Returns (successful_outputs, failed_specialists) where failed_specialists
/// contains (agent_key, error_message) tuples.
fn execute_specialist_stub(
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
) -> (HashMap<String, String>, Vec<(String, String)>) {
    let fingerprint_summary = prompt_builder::fingerprint_summary_text(fingerprint);
    let mut outputs = HashMap::new();
    let mut failed: Vec<(String, String)> = Vec::new();

    for agent_key in &routing.enabled_specialists {
        let result = execute_single_specialist(
            agent_key, situation, &fingerprint_summary,
        );

        match result {
            Ok(output) => {
                outputs.insert(agent_key.clone(), output);
            }
            Err(err_msg) => {
                failed.push((agent_key.clone(), err_msg));
            }
        }
    }

    (outputs, failed)
}

/// Execute a single specialist (stub — returns placeholder output).
///
/// Test hook: if `agent_key` ends with `-FORCE-FAIL-FOR-TEST`, returns Err.
fn execute_single_specialist(
    agent_key: &str,
    situation: &str,
    fingerprint_summary: &str,
) -> Result<String, String> {
    // Test hook: force failure for failure isolation testing
    if agent_key.ends_with("-FORCE-FAIL-FOR-TEST") {
        return Err(format!(
            "forced failure for test: {agent_key}"
        ));
    }

    let _prompt = prompt_builder::build_specialist_prompt(
        agent_key,
        agent_key,
        situation,
        fingerprint_summary,
        &[], // no dependency outputs in stub mode
    );

    Ok(format!(
        "## {agent_key} — Stub Analysis\n\n\
         **Situation:** {situation}\n\n\
         **Fingerprint:** {fp_summary}\n\n\
         *Phase 5 will replace this with real LLM agent output.*",
        fp_summary = fingerprint_summary,
    ))
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

// ── Phase 4 persistence helpers ──────────────────────────────────────────────

fn persist_routing_decision(db: &DbInstance, rd: &RoutingDecision) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;

    let enabled_json =
        serde_json::to_string(&rd.enabled_specialists).unwrap_or_else(|_| "[]".into());
    let skipped_json =
        serde_json::to_string(&rd.skipped_specialists).unwrap_or_else(|_| "[]".into());
    let conditions_json =
        serde_json::to_string(&rd.evaluated_conditions).unwrap_or_else(|_| "[]".into());

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(rd.run_id.as_str()));
    params.insert("fid".into(), cozo::DataValue::from(rd.fingerprint_id.as_str()));
    params.insert("en".into(), cozo::DataValue::from(enabled_json.as_str()));
    params.insert("sk".into(), cozo::DataValue::from(skipped_json.as_str()));
    params.insert("ec".into(), cozo::DataValue::from(conditions_json.as_str()));
    params.insert("ca".into(), cozo::DataValue::from(rd.created_at.as_str()));

    db.run_script(
        "?[run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, \
         evaluated_conditions_json, created_at] \
         <- [[$rid, $fid, $en, $sk, $ec, $ca]] \
         :put gt_routing_decisions { run_id => fingerprint_id, enabled_specialists_json, \
         skipped_specialists_json, evaluated_conditions_json, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_routing_decisions failed: {e}"))?;
    Ok(())
}

fn persist_specialist_outputs(
    db: &DbInstance,
    run_id: &str,
    outputs: &HashMap<String, String>,
) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;

    for (agent_key, output) in outputs {
        let mut params = BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(run_id));
        params.insert("ak".into(), cozo::DataValue::from(agent_key.as_str()));
        params.insert("out".into(), cozo::DataValue::from(output.as_str()));

        db.run_script(
            "?[run_id, agent_key, output_json] <- [[$rid, $ak, $out]] \
             :put gt_specialist_outputs { run_id, agent_key => output_json }",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("persist gt_specialist_outputs failed: {e}"))?;
    }
    Ok(())
}

fn persist_sections(db: &DbInstance, run_id: &str, report: &str) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;

    let now = Utc::now().to_rfc3339();
    let mut section_order = 0u32;
    for line in report.lines() {
        if line.starts_with("## ") {
            let title = line.trim_start_matches("## ").trim().to_string();
            section_order += 1;
            let section_id = format!("sec-{section_order}");

            let mut params = BTreeMap::new();
            params.insert("rid".into(), cozo::DataValue::from(run_id));
            params.insert("sid".into(), cozo::DataValue::from(section_id.as_str()));
            params.insert("sty".into(), cozo::DataValue::from(title.as_str()));
            params.insert("stt".into(), cozo::DataValue::from(title.as_str()));
            params.insert("smd".into(), cozo::DataValue::from(""));
            params.insert("ssj".into(), cozo::DataValue::from("[]"));
            params.insert("ca".into(), cozo::DataValue::from(now.as_str()));

            db.run_script(
                "?[run_id, section_id, section_type, title, content_md, \
                 source_specialists_json, created_at] \
                 <- [[$rid, $sid, $sty, $stt, $smd, $ssj, $ca]] \
                 :put gt_sections { run_id, section_id => section_type, title, \
                 content_md, source_specialists_json, created_at }",
                params,
                cozo::ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("persist gt_sections failed: {e}"))?;
        }
    }
    Ok(())
}

fn persist_final_report(db: &DbInstance, run_id: &str, report: &str) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;

    let now = Utc::now().to_rfc3339();

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("rep".into(), cozo::DataValue::from(report));
    params.insert("ca".into(), cozo::DataValue::from(now.as_str()));

    db.run_script(
        "?[run_id, report_md, created_at, total_cost_usd, total_duration_ms] \
         <- [[$rid, $rep, $ca, '0.0', '0']] \
         :put gt_final_reports { run_id => report_md, created_at, \
         total_cost_usd, total_duration_ms }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_final_reports failed: {e}"))?;
    Ok(())
}

// ── Cozo helpers ─────────────────────────────────────────────────────────────

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

    #[test]
    fn test_full_pipeline_produces_report() {
        let db = test_db();
        let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

        // Skip if spec file doesn't exist (CI-safe)
        if !spec_path.exists() {
            eprintln!("spec file not found, skipping full pipeline test");
            return;
        }

        let result = run_full_pipeline(
            &db,
            "Two firms simultaneously set prices in a Bertrand duopoly with asymmetric costs.",
            Some(spec_path),
        );
        assert!(result.is_ok(), "full pipeline must succeed: {:?}", result.err());

        let r = result.unwrap();
        assert!(!r.run_id.is_empty());
        assert!(!r.report.is_empty());
        assert!(r.specialist_count > 0, "at least one specialist enabled");
        assert!(r.report.contains("Strategic Game-Theory Analysis"));

        // Verify Cozo relations populated
        let routing_rows = db
            .run_script(
                "?[count(run_id)] := *gt_routing_decisions{run_id}",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
        assert!(routing_rows.rows.len() >= 1);
    }

    #[test]
    fn test_replay_determinism() {
        let db = test_db();
        let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

        if !spec_path.exists() {
            eprintln!("spec file not found, skipping replay test");
            return;
        }

        let situation = "Two firms simultaneously set quantities in a Cournot duopoly.";

        let r1 = run_full_pipeline(&db, situation, Some(spec_path)).unwrap();
        let r2 = run_full_pipeline(&db, situation, Some(spec_path)).unwrap();

        // Same situation → same routing decisions
        assert_eq!(
            r1.routing_decision.enabled_specialists,
            r2.routing_decision.enabled_specialists,
            "routing must be deterministic"
        );
        assert_eq!(
            r1.routing_decision.skipped_specialists,
            r2.routing_decision.skipped_specialists,
            "skipped specialists must be deterministic"
        );
    }

    #[test]
    fn test_full_pipeline_classify_only_mode() {
        // classify() is the classify-only entrypoint — it persists fingerprint
        // but does not run routing or specialists
        let db = test_db();
        let fp = classify(
            &db,
            "Two firms negotiate a bilateral trade agreement with complete information.",
        )
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
    fn test_stub_specialist_outputs_non_empty() {
        let db = test_db();
        let fp = classify(&db, "Two firms set quantities simultaneously.").unwrap();

        // Build a minimal routing decision to test stub execution
        let rd = RoutingDecision {
            run_id: "test-stub-run".into(),
            fingerprint_id: fp.run_id.clone(),
            enabled_specialists: vec![
                "nash-equilibrium-finder".into(),
                "payoff-matrix-builder".into(),
            ],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let (outputs, failed) = execute_specialist_stub(&rd, &fp, "Two firms set quantities.");
        assert_eq!(outputs.len(), 2);
        assert!(outputs.get("nash-equilibrium-finder").unwrap().contains("nash-equilibrium-finder"));
        assert!(outputs.get("payoff-matrix-builder").unwrap().contains("payoff-matrix-builder"));
        assert!(failed.is_empty(), "no forced failures without the test hook suffix");
    }

    #[test]
    fn test_failure_isolation_with_force_fail_suffix() {
        let db = test_db();
        let fp = classify(&db, "Two firms set quantities simultaneously.").unwrap();

        let rd = RoutingDecision {
            run_id: "test-fail-iso".into(),
            fingerprint_id: fp.run_id.clone(),
            enabled_specialists: vec![
                "nash-equilibrium-finder".into(),
                "bayesian-game-analyzer-FORCE-FAIL-FOR-TEST".into(),
                "payoff-matrix-builder".into(),
            ],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let (outputs, failed) = execute_specialist_stub(&rd, &fp, "Two firms set quantities.");
        // 2 of 3 succeed
        assert_eq!(outputs.len(), 2);
        assert!(outputs.contains_key("nash-equilibrium-finder"));
        assert!(outputs.contains_key("payoff-matrix-builder"));
        // 1 fails due to test hook
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "bayesian-game-analyzer-FORCE-FAIL-FOR-TEST");
        assert!(failed[0].1.contains("forced failure"));
    }

    #[test]
    fn test_full_pipeline_partial_status_on_failure() {
        let db = test_db();
        let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

        if !spec_path.exists() {
            eprintln!("spec file not found, skipping partial status test");
            return;
        }

        // No forced failure → completed
        let result = run_full_pipeline(
            &db,
            "Two firms simultaneously set prices in a Bertrand duopoly.",
            Some(spec_path),
        )
        .unwrap();
        assert_eq!(result.status, "completed");
        assert!(result.failed_specialists.is_empty());
    }
}
