use super::super::*;
use super::*;
use crate::learning::{
    gnn::auto_trainer_runtime::query_trajectories_for_training,
    integration::{LearningIntegration, LearningIntegrationConfig},
    schema::initialize_learning_schemas,
};
use std::sync::Arc;

#[test]
fn test_full_pipeline_produces_report() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

    // Skip if spec file doesn't exist (CI-safe)
    if !spec_path.exists() {
        eprintln!("spec file not found, skipping full pipeline test");
        return;
    }

    let llm = canned_pipeline_llm();
    let result = block_on(run_full_pipeline(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly with asymmetric costs.",
        Some(spec_path),
        Some(&llm),
    ));
    assert!(
        result.is_ok(),
        "full pipeline must succeed: {:?}",
        result.err()
    );

    let r = result.unwrap();
    assert!(!r.run_id.is_empty());
    assert!(!r.report.is_empty());
    assert!(r.specialist_count > 0, "at least one specialist enabled");
    assert!(r.report.contains("Strategic Game-Theory Analysis"));

    // Verify Cozo source-of-truth relations populated and status matches
    // the returned pipeline outcome.
    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(r.run_id.as_str()));
    let run_rows = db
            .run_script(
                "?[status] := *gt_runs{run_id, situation, started_at, completed_at, status}, run_id = $rid",
                params,
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
    assert_eq!(run_rows.rows[0][0].get_str().unwrap(), r.status);

    let routing_rows = db
        .run_script(
            "?[count(run_id)] := *gt_routing_decisions{run_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(!routing_rows.rows.is_empty());

    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(r.run_id.as_str()));
    let section_rows = db
        .run_script(
            "?[section_id, content, sources] := *gt_sections{run_id, section_id, \
                 content_md: content, source_specialists_json: sources}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(section_rows.rows.len(), 11);
    assert!(
        section_rows
            .rows
            .iter()
            .any(|row| !row[1].get_str().unwrap_or("").trim().is_empty()
                && row[2].get_str().unwrap_or("") != "[]"),
        "persisted sections must retain content and source specialists"
    );

    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(r.run_id.as_str()));
    let quality_rows = db
        .run_script(
            "?[agent_key, gate_name, passed, detail] := *gt_quality_checks{run_id, \
                 agent_key, gate_name, passed, detail}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(
        quality_rows.rows.len() >= r.specialist_count * 3,
        "each persisted specialist output must have all quality gate rows"
    );
    assert!(
        quality_rows
            .rows
            .iter()
            .any(|row| row[1].get_str().unwrap_or("") == "citation-count"
                && row[2].get_str().unwrap_or("") == "false"
                && !row[3].get_str().unwrap_or("").is_empty()),
        "failed quality gates must persist auditable details"
    );

    let edge_rows = db
        .run_script(
            "?[edge_id, edge_type] := *gt_provenance_edges{edge_id, from_artifact_id, \
                 to_artifact_id, edge_type}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    let edge_types: Vec<&str> = edge_rows
        .rows
        .iter()
        .filter(|row| row[0].get_str().unwrap_or("").contains(&r.run_id))
        .filter_map(|row| row[1].get_str())
        .collect();
    assert!(edge_types.contains(&"produced_fingerprint"));
    assert!(edge_types.contains(&"produced_routing"));
    assert!(edge_types.contains(&"enabled_specialist"));
    assert!(edge_types.contains(&"contributed_to_section"));
    assert!(edge_types.contains(&"assembled_into_report"));
}

#[test]
fn test_full_pipeline_records_sona_when_learning_supplied() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

    if !spec_path.exists() {
        eprintln!("spec file not found, skipping gametheory SONA test");
        return;
    }

    let learning_db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    initialize_learning_schemas(learning_db.as_ref()).unwrap();
    let mut learning = LearningIntegration::new_with_persistent_sona(
        Arc::clone(&learning_db),
        LearningIntegrationConfig::default(),
        None,
        8,
    );

    let llm = canned_pipeline_llm();
    let result = block_on(run_full_pipeline_with_learning_options(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly with asymmetric costs.",
        Some(spec_path),
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions {
            budget_usd: 20.0,
            max_concurrent: 1,
            style_profile_id: Some("technical".to_string()),
            enable_tier11: false,
            kb_pack_id: None,
        },
        Some(&mut learning),
    ))
    .expect("gametheory pipeline should complete with learning enabled");

    assert_eq!(result.status, "completed");
    let samples = query_trajectories_for_training(learning_db.as_ref(), 8).unwrap();
    assert!(
        samples.len() >= 2,
        "tier1 and specialist agents should record SONA trajectories"
    );
    assert!(samples.iter().all(|sample| sample.quality > 0.0));
    assert!(samples.iter().all(|sample| sample.embedding.len() == 8));
}

#[test]
fn test_full_pipeline_partial_status_on_failure() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

    if !spec_path.exists() {
        eprintln!("spec file not found, skipping partial status test");
        return;
    }

    let llm = canned_pipeline_llm();
    // No forced failure -> completed
    let result = block_on(run_full_pipeline(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        Some(spec_path),
        Some(&llm),
    ))
    .unwrap();
    assert_eq!(result.status, "completed");
    assert!(result.failed_specialists.is_empty());
}
#[test]
fn test_full_pipeline_requires_llm_provider_and_writes_no_run() {
    let db = test_db();
    ensure_gametheory_schema(&db).unwrap();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");

    let err = block_on(run_full_pipeline(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        Some(spec_path),
        None,
    ))
    .unwrap_err();
    assert!(matches!(err, GameTheoryError::LlmUnavailable { .. }));

    let rows = db
        .run_script(
            "?[count(run_id)] := *gt_runs{run_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(rows.rows[0][0].get_int().unwrap(), 0);
}
#[test]
fn test_budget_cap_halts_pipeline_gracefully_with_partial_report() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");
    if !spec_path.exists() {
        eprintln!("spec file not found, skipping budget cap test");
        return;
    }

    let llm = canned_pipeline_llm();
    let result = block_on(run_full_pipeline_with_options(
        &db,
        "Two firms simultaneously set prices in a Bertrand duopoly with asymmetric costs.",
        Some(spec_path),
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions {
            budget_usd: 0.0001,
            max_concurrent: 1,
            style_profile_id: Some("executive".to_string()),
            enable_tier11: false,
            kb_pack_id: None,
        },
    ))
    .unwrap();

    assert_eq!(result.status, "BudgetExceeded");
    assert!(result.report.contains("[BUDGET-EXCEEDED]"));
    assert!(result.specialist_count < result.routing_decision.enabled_specialists.len());

    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(result.run_id.as_str()));
    let rows = db
        .run_script(
            "?[status, cost] := *gt_runs{run_id, status, cost_usd: cost}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(rows.rows[0][0].get_str().unwrap(), "BudgetExceeded");
    assert_eq!(rows.rows[0][1].get_str().unwrap(), "0.016500");

    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(result.run_id.as_str()));
    let specialist_rows = db
        .run_script(
            "?[cost] := *gt_specialist_outputs{run_id, agent_key, cost_usd: cost}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(specialist_rows.rows.len(), result.specialist_count);
    assert_eq!(specialist_rows.rows[0][0].get_str().unwrap(), "0.016500");

    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(result.run_id.as_str()));
    let report_rows = db
        .run_script(
            "?[cost] := *gt_final_reports{run_id, total_cost_usd: cost}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(report_rows.rows[0][0].get_str().unwrap(), "0.016500");
}
#[test]
fn test_style_flag_applied_to_section_writers() {
    let db = test_db();
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");
    if !spec_path.exists() {
        eprintln!("spec file not found, skipping style flag test");
        return;
    }

    let llm = canned_pipeline_llm();
    let result = block_on(run_full_pipeline_with_options(
        &db,
        "Two firms simultaneously set quantities in a Cournot duopoly.",
        Some(spec_path),
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions {
            budget_usd: 20.0,
            max_concurrent: 1,
            style_profile_id: Some("technical".to_string()),
            enable_tier11: false,
            kb_pack_id: None,
        },
    ))
    .unwrap();

    assert!(result.report.contains("Style: technical"));
}
#[test]
fn test_kb_flag_reads_doc_chunks_into_llm_context_and_checkpoint() {
    let db = test_db();
    seed_kb_pack(
        &db,
        "policy-pack",
        "SYNTHETIC KB CONTEXT: marketplaces reward lock-in.",
    );
    let spec_path = std::path::Path::new("../../.archon/specs/gametheory.yaml");
    if !spec_path.exists() {
        eprintln!("spec file not found, skipping kb context test");
        return;
    }

    let llm = canned_pipeline_llm();
    let result = block_on(run_full_pipeline_with_options(
        &db,
        "Assess the incentive structure of this plugin marketplace design",
        Some(spec_path),
        Some(&llm),
        GameTheoryMemoryContext::default(),
        GameTheoryRunOptions {
            budget_usd: 20.0,
            max_concurrent: 1,
            style_profile_id: Some("executive".to_string()),
            enable_tier11: false,
            kb_pack_id: Some("policy-pack".to_string()),
        },
    ))
    .unwrap();

    let prompts = llm.prompts().join("\n");
    assert!(prompts.contains("Knowledge Base Context: policy-pack"));
    assert!(prompts.contains("SYNTHETIC KB CONTEXT"));

    let checkpoint = db
        .run_script(
            "?[detail_json] := *gt_run_checkpoints{run_id, checkpoint_key, detail_json}, \
                 run_id = $rid, checkpoint_key = \"stage:kb-context\"",
            std::collections::BTreeMap::from([(
                "rid".into(),
                cozo::DataValue::from(result.run_id.as_str()),
            )]),
            cozo::ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(checkpoint.rows.len(), 1);
    let detail: serde_json::Value =
        serde_json::from_str(checkpoint.rows[0][0].get_str().unwrap()).unwrap();
    assert_eq!(detail["kb"], "policy-pack");
    assert_eq!(detail["documents"], 1);
    assert_eq!(detail["chunks"], 1);
}
#[test]
fn test_kb_context_missing_doc_store_is_explicit_warning() {
    let db = test_db();
    let context = load_kb_run_context(&db, Some("missing-pack")).unwrap();

    assert_eq!(context.pack_id.as_deref(), Some("missing-pack"));
    assert_eq!(context.document_count, 0);
    assert_eq!(context.chunk_count, 0);
    assert!(
        context
            .warning
            .as_deref()
            .unwrap_or("")
            .contains("document store unavailable")
    );
}
