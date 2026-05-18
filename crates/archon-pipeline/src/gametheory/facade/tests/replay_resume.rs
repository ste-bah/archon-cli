use super::super::loaders::load_stored_routing;
use super::super::*;
use super::*;

#[test]
fn test_replay_determinism() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

    if !spec_path.exists() {
        eprintln!("spec file not found, skipping replay test");
        return;
    }

    let situation = "Two firms simultaneously set quantities in a Cournot duopoly.";

    let llm1 = canned_pipeline_llm();
    let llm2 = canned_pipeline_llm();
    let r1 = block_on(run_full_pipeline(
        &db,
        situation,
        Some(spec_path),
        Some(&llm1),
    ))
    .unwrap();
    let r2 = block_on(run_full_pipeline(
        &db,
        situation,
        Some(spec_path),
        Some(&llm2),
    ))
    .unwrap();

    // Same situation → same routing decisions
    assert_eq!(
        r1.routing_decision.enabled_specialists, r2.routing_decision.enabled_specialists,
        "routing must be deterministic"
    );
    assert_eq!(
        r1.routing_decision.skipped_specialists, r2.routing_decision.skipped_specialists,
        "skipped specialists must be deterministic"
    );
}
#[test]
fn test_replay_routing_persists_refreshed_decision() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");
    if !spec_path.exists() {
        eprintln!("spec file not found, skipping replay routing test");
        return;
    }

    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();
    let rd = replay_routing_from_stored_fingerprint(&db, &fp.run_id, Some(spec_path)).unwrap();
    assert!(!rd.enabled_specialists.is_empty());

    let rows = db
        .run_script(
            "?[enabled] := *gt_routing_decisions{run_id, fingerprint_id, \
                 enabled_specialists_json: enabled, skipped_specialists_json, \
                 evaluated_conditions_json, created_at}, run_id = $rid",
            {
                let mut p = std::collections::BTreeMap::new();
                p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                p
            },
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(rows.rows.len(), 1);
    assert!(
        rows.rows[0][0]
            .get_str()
            .unwrap()
            .contains("game-classifier")
    );
}
#[test]
fn test_replay_single_specialist_updates_source_of_truth() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();
    let llm = canned_pipeline_llm();

    let replayed = block_on(replay_single_specialist(
        &db,
        &fp.run_id,
        "nash-equilibrium-finder",
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions::default(),
    ))
    .unwrap();
    assert_eq!(replayed.status, "completed");
    assert!(replayed.output_summary.contains("specialist output"));

    let rows = db
            .run_script(
                "?[output, status] := *gt_specialist_outputs{run_id, agent_key, \
                 output_json: output, status}, run_id = $rid, agent_key = 'nash-equilibrium-finder'",
                {
                    let mut p = std::collections::BTreeMap::new();
                    p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                    p
                },
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][1].get_str().unwrap(), "completed");
    assert!(
        rows.rows[0][0]
            .get_str()
            .unwrap()
            .contains("specialist output")
    );
    assert!(
        !rows.rows[0][0]
            .get_str()
            .unwrap()
            .contains("Fixture Analysis")
    );
}
#[test]
fn test_replay_single_specialist_requires_provider_and_writes_no_output() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();

    let err = block_on(replay_single_specialist(
        &db,
        &fp.run_id,
        "nash-equilibrium-finder",
        None,
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions::default(),
    ))
    .unwrap_err();
    assert!(matches!(err, GameTheoryError::LlmUnavailable { .. }));

    let rows = db
        .run_script(
            "?[count(agent_key)] := *gt_specialist_outputs{run_id, agent_key}, run_id = $rid",
            {
                let mut p = std::collections::BTreeMap::new();
                p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                p
            },
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(rows.rows[0][0].get_int().unwrap(), 0);
}
#[test]
fn test_specialist_completion_writes_checkpoint_source_of_truth() {
    let db = test_db();
    ensure_gametheory_schema(&db).unwrap();
    let mut outputs = HashMap::new();
    outputs.insert(
        "nash-equilibrium-finder".to_string(),
        "analysis".to_string(),
    );
    persist_specialist_outputs(&db, "run-checkpoint", &outputs, &HashMap::new()).unwrap();

    let rows = db
            .run_script(
                "?[checkpoint_type, status, detail] := *gt_run_checkpoints{run_id, checkpoint_key, checkpoint_type, status, detail_json: detail}, \
                 run_id = 'run-checkpoint', checkpoint_key = 'specialist:nash-equilibrium-finder'",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][0].get_str().unwrap(), "specialist");
    assert_eq!(rows.rows[0][1].get_str().unwrap(), "completed");
    assert!(
        rows.rows[0][2]
            .get_str()
            .unwrap()
            .contains("nash-equilibrium-finder")
    );
}
#[test]
fn test_resume_run_completes_missing_specialists_from_checkpoint() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();
    let routing = RoutingDecision {
        run_id: fp.run_id.clone(),
        fingerprint_id: fp.run_id.clone(),
        enabled_specialists: vec![
            "nash-equilibrium-finder".into(),
            "payoff-matrix-builder".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: fp.created_at.clone(),
    };
    persist_routing_decision(&db, &routing).unwrap();
    let mut completed = HashMap::new();
    completed.insert(
        "nash-equilibrium-finder".to_string(),
        "already completed".to_string(),
    );
    let mut costs = HashMap::new();
    costs.insert("nash-equilibrium-finder".to_string(), 1.25);
    persist_specialist_outputs(&db, &fp.run_id, &completed, &costs).unwrap();
    update_gt_run_status(
        &db,
        &fp.run_id,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        &fp.created_at,
        "",
        "InProgress",
        0.0,
    )
    .unwrap();

    let in_progress = list_in_progress_runs(&db).unwrap();
    assert_eq!(in_progress.len(), 1);

    let llm = canned_pipeline_llm();
    let result = block_on(resume_run_from_checkpoint(
        &db,
        &fp.run_id,
        None,
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions::default(),
    ))
    .unwrap();
    assert_eq!(result.resumed_specialists, 1);
    assert_eq!(result.skipped_completed_specialists, 1);
    assert_eq!(result.failed_specialists, 0);
    assert!((result.total_cost_usd - 1.2665).abs() < 0.000001);

    let rows = db
            .run_script(
                "?[agent_key, status] := *gt_specialist_outputs{run_id, agent_key, status}, run_id = $rid",
                {
                    let mut p = std::collections::BTreeMap::new();
                    p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                    p
                },
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
    let completed_count = rows
        .rows
        .iter()
        .filter(|row| row[1].get_str() == Some("completed"))
        .count();
    assert_eq!(completed_count, 2);
}
#[test]
fn test_resume_rejects_non_in_progress_run() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();
    let err = block_on(resume_run_from_checkpoint(
        &db,
        &fp.run_id,
        None,
        None,
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions::default(),
    ))
    .unwrap_err();
    assert!(err.to_string().contains("not resumable"));
}
#[test]
fn test_resume_failure_marks_run_partial_in_source_of_truth() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();
    let routing = RoutingDecision {
        run_id: fp.run_id.clone(),
        fingerprint_id: fp.run_id.clone(),
        enabled_specialists: vec!["game-tree-builder-FORCE-FAIL-FOR-TEST".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: fp.created_at.clone(),
    };
    persist_routing_decision(&db, &routing).unwrap();
    update_gt_run_status(
        &db,
        &fp.run_id,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        &fp.created_at,
        "",
        "InProgress",
        0.0,
    )
    .unwrap();

    let llm = canned_specialist_llm();
    let result = block_on(resume_run_from_checkpoint(
        &db,
        &fp.run_id,
        None,
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions::default(),
    ))
    .unwrap();
    assert_eq!(result.status, "partial");
    assert_eq!(result.failed_specialists, 1);

    let rows = db
            .run_script(
                "?[status] := *gt_runs{run_id, situation, started_at, completed_at, status}, run_id = $rid",
                {
                    let mut p = std::collections::BTreeMap::new();
                    p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                    p
                },
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
    assert_eq!(rows.rows[0][0].get_str().unwrap(), "partial");

    let checkpoints = db
            .run_script(
                "?[status, detail] := *gt_run_checkpoints{run_id, checkpoint_key, status, detail_json: detail}, \
                 run_id = $rid, checkpoint_key = 'stage:resume-complete'",
                {
                    let mut p = std::collections::BTreeMap::new();
                    p.insert("rid".into(), cozo::DataValue::from(fp.run_id.as_str()));
                    p
                },
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
    assert_eq!(checkpoints.rows[0][0].get_str().unwrap(), "partial");
    assert!(
        checkpoints.rows[0][1]
            .get_str()
            .unwrap()
            .contains("\"failed\":1")
    );
}
#[test]
fn test_resume_fallback_routing_applies_tier11_policy_gate() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        None,
    ))
    .unwrap();
    update_gt_run_status(
        &db,
        &fp.run_id,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        &fp.created_at,
        "",
        "InProgress",
        0.0,
    )
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let spec_path = dir.path().join("gametheory.yaml");
    std::fs::write(
        &spec_path,
        r#"
version: "test"
spec_id: "tier11-resume-test"
tiers:
  - id: 11
    name: "Tier 11"
    concurrency_cap: 1
    agents:
      - key: "cohesion-discipline-devotion-auditor"
        mandatory: true
        depends_on: []
"#,
    )
    .unwrap();

    let result = block_on(resume_run_from_checkpoint(
        &db,
        &fp.run_id,
        Some(&spec_path),
        None,
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions::default(),
    ))
    .unwrap();
    assert_eq!(result.resumed_specialists, 0);

    let routing = load_stored_routing(&db, &fp.run_id).unwrap().unwrap();
    assert!(routing.enabled_specialists.is_empty());
    assert!(routing.skipped_specialists.iter().any(|(key, reason)| key
        == "cohesion-discipline-devotion-auditor"
        && reason.contains("Tier 11 disabled")));
}
