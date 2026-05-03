use archon_pipeline::gametheory;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::gametheory_inspect::{
    render_inspect_artifact, render_inspect_fingerprint, render_inspect_routing,
    render_list_agents, render_show, render_status,
};

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-gt-cli-inspect-{}.db", uuid::Uuid::new_v4());
    DbInstance::new("sqlite", &path, "").unwrap()
}

fn seed_run(db: &DbInstance) {
    gametheory::schema::ensure_gametheory_schema(db).unwrap();
    let fp = serde_json::json!({
        "run_id": "gt-synth",
        "cooperation": axis("non-cooperative"),
        "payoff_sum": axis("non-zero-sum"),
        "symmetry": axis("asymmetric"),
        "timing": axis("simultaneous"),
        "perfect_info": axis("imperfect"),
        "complete_info": axis("complete"),
        "cardinality": axis("2-player"),
        "strategy_space": axis("continuous"),
        "horizon": axis("one-shot"),
        "primary_family": "Bertrand competition",
        "nearest_classic": "Bertrand duopoly",
        "shadow_games": [],
        "hidden_game_scan": null,
        "ambiguities": [],
        "created_at": "2026-05-03T00:00:00Z"
    })
    .to_string();
    put_run(db, &fp);
    put_artifacts(db);
}

fn axis(value: &str) -> serde_json::Value {
    serde_json::json!({"value": value, "confidence": "high", "rationale": "synthetic"})
}

fn put_run(db: &DbInstance, fingerprint_json: &str) {
    let mut params = std::collections::BTreeMap::new();
    params.insert("fp".into(), DataValue::from(fingerprint_json));
    params.insert(
        "enabled".into(),
        DataValue::from("[\"nash-equilibrium-finder\"]"),
    );
    params.insert(
        "skipped".into(),
        DataValue::from("[[\"auction-strategist\",\"condition false\"]]"),
    );
    params.insert(
        "conditions".into(),
        DataValue::from("[[\"timing == 'simultaneous'\",true]]"),
    );
    db.run_script(
        "?[run_id, situation, started_at, completed_at, status, cost_usd] <- \
         [['gt-synth', 'Synthetic price game', '2026-05-03T00:00:00Z', \
         '2026-05-03T00:00:01Z', 'completed', '0.123000']] \
         :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[run_id, fingerprint_json, primary_family, created_at] <- \
         [['gt-synth', $fp, 'Bertrand competition', '2026-05-03T00:00:00Z']] \
         :put gt_fingerprints { run_id => fingerprint_json, primary_family, created_at }",
        params.clone(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, \
         evaluated_conditions_json, created_at] <- [['gt-synth', 'gt-synth', $enabled, \
         $skipped, $conditions, '2026-05-03T00:00:00Z']] \
         :put gt_routing_decisions { run_id => fingerprint_id, enabled_specialists_json, \
         skipped_specialists_json, evaluated_conditions_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();
}

fn put_artifacts(db: &DbInstance) {
    db.run_script(
        "?[run_id, agent_key, output_json, status, started_at, completed_at, duration_ms, cost_usd] <- \
         [['gt-synth', 'nash-equilibrium-finder', 'Synthetic specialist verdict', \
         'completed', '2026-05-03T00:00:00Z', '2026-05-03T00:00:01Z', '1000', '0.004200']] \
         :put gt_specialist_outputs { run_id, agent_key => output_json, status, started_at, \
         completed_at, duration_ms, cost_usd }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[run_id, section_id, section_type, title, content_md, source_specialists_json, created_at] <- \
         [['gt-synth', 'sec-1', 'Executive Summary', 'Executive Summary', \
         'Synthetic section content', '[\"nash-equilibrium-finder\"]', '2026-05-03T00:00:01Z']] \
         :put gt_sections { run_id, section_id => section_type, title, content_md, \
         source_specialists_json, created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[run_id, report_md, created_at, total_cost_usd, total_duration_ms] <- \
         [['gt-synth', '# Final Report\n\nSynthetic final report body', \
         '2026-05-03T00:00:01Z', '0.123000', '1000']] \
         :put gt_final_reports { run_id => report_md, created_at, total_cost_usd, total_duration_ms }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
}

#[test]
fn test_status_command_reads_gt_runs_source_of_truth() {
    let db = test_db();
    seed_run(&db);

    let status = render_status(&db, Some("gt-synth")).unwrap();
    assert!(status.contains("Status:    completed"));
    assert!(status.contains("Cost USD:  $0.123000"));

    let summary = render_status(&db, None).unwrap();
    assert!(summary.contains("Total Runs: 1"));
    assert!(summary.contains("completed: 1"));
}

#[test]
fn test_inspection_surfaces_render_synthetic_artifacts() {
    let db = test_db();
    seed_run(&db);

    assert!(
        render_show(&db, "gt-synth")
            .unwrap()
            .contains("Synthetic price game")
    );
    assert!(
        render_inspect_fingerprint(&db, "gt-synth")
            .unwrap()
            .contains("Bertrand competition")
    );
    assert!(
        render_inspect_routing(&db, "gt-synth")
            .unwrap()
            .contains("nash-equilibrium-finder")
    );
    assert!(
        render_inspect_artifact(&db, "specialist:gt-synth:nash-equilibrium-finder")
            .unwrap()
            .contains("Synthetic specialist verdict")
    );
    assert!(
        render_inspect_artifact(&db, "section:gt-synth:sec-1")
            .unwrap()
            .contains("Synthetic section content")
    );
    assert!(
        render_inspect_artifact(&db, "final-report:gt-synth")
            .unwrap()
            .contains("Synthetic final report body")
    );
}

#[test]
fn test_list_agents_filters_by_tier() {
    let output = render_list_agents(Some(2)).unwrap();
    assert!(output.contains("Tier Filter: 2"));
    assert!(output.contains("nash-equilibrium-finder"));
    assert!(!output.contains("auction-strategist"));
}

#[test]
fn test_inspect_unknown_artifact_reports_not_found() {
    let db = test_db();
    seed_run(&db);

    let output = render_inspect_artifact(&db, "missing-artifact").unwrap();
    assert!(output.contains("not found"));
    assert!(output.contains("Supported formats"));
}
