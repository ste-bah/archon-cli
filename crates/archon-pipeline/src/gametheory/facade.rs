//! Game-theory orchestration entrypoints.
//!
//! `classify` — Tier 1 classification only (fingerprint + persistence).
//! `run_full_pipeline` — classify → route → specialist DAG → final report.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use cozo::DbInstance;

use super::errors::GameTheoryError;
use super::final_stage;
use super::fingerprint::GameTheoryFingerprint;
use super::quality;
use super::routing::{evaluate_routing, load_spec, resolve_spec_path};
use super::schema::ensure_gametheory_schema;
use super::spec::build_specialist_spec;
use crate::runner::LlmClient;

mod costs;
mod fallback;
mod kb_context;
mod loaders;
mod memory_context;
mod persistence;
mod policy;
mod replay;
mod specialists;
mod tier1;
mod types;

use fallback::keyword_fallback_fingerprint;
use kb_context::{load_kb_run_context, situation_with_kb_context};
pub use memory_context::{GameTheoryMemoryContext, MemoryRecallAudit};
use persistence::{
    insert_gt_fingerprint, insert_gt_run, persist_final_report, persist_provenance_edges_for_run,
    persist_quality_checks, persist_routing_decision, persist_run_checkpoint, persist_sections,
    persist_specialist_outputs, persist_tier_checkpoints, update_gt_run_status,
};
use policy::{apply_policy_gates_to_routing, build_dependency_map};
pub use replay::{
    list_in_progress_runs, replay_routing_from_stored_fingerprint, replay_single_specialist,
    resume_run_from_checkpoint,
};
use specialists::execute_specialists_real_with_options;
#[cfg(test)]
use specialists::{execute_specialists_real, execute_test_specialist_fixture};
use tier1::execute_tier1_real;
pub use types::{
    FullPipelineResult, GameTheoryRunOptions, InProgressRun, ReplaySpecialistResult,
    ResumeRunResult,
};

const TAG_GAMETHEORY_PIPELINE: &str = "gametheory-pipeline";
const TIER1_MEMORY_AGENT_KEYS: &[&str] = &[
    "game-classifier",
    "payoff-elicitor",
    "strategy-space-enumerator",
    "information-structure-mapper",
];

fn require_llm<'a>(
    llm: Option<&'a dyn LlmClient>,
    operation: &str,
) -> Result<&'a dyn LlmClient, GameTheoryError> {
    llm.ok_or_else(|| GameTheoryError::LlmUnavailable {
        operation: operation.to_string(),
    })
}

/// Run Tier 1 classification on a situation and persist the fingerprint.
///
/// When `llm` is provided, attempts real LLM-backed classification first.
/// Classify-only calls may use a labelled keyword fingerprint when LLM auth is
/// unavailable; full pipeline calls disable that fallback.
///
/// Returns the generated fingerprint after persistence.
pub async fn classify(
    db: &DbInstance,
    situation: &str,
    llm: Option<&dyn LlmClient>,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    let (fingerprint, _) = classify_internal(
        db,
        situation,
        llm,
        &GameTheoryMemoryContext::default(),
        true,
    )
    .await?;
    Ok(fingerprint)
}

async fn classify_internal(
    db: &DbInstance,
    situation: &str,
    llm: Option<&dyn LlmClient>,
    memory_ctx: &GameTheoryMemoryContext,
    allow_keyword_fallback: bool,
) -> Result<(GameTheoryFingerprint, Vec<MemoryRecallAudit>), GameTheoryError> {
    let situation = situation.trim();
    if situation.is_empty() {
        return Err(GameTheoryError::EmptySituation);
    }

    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    let run_id = format!(
        "gt-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );
    let now = Utc::now().to_rfc3339();

    // Insert run with status "running"
    insert_gt_run(db, &run_id, situation, &now, "running").map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;

    // Attempt real Tier 1 classification, fall back to keyword analysis
    let mut memory_audits = Vec::new();
    let fingerprint = if let Some(llm_client) = llm {
        match execute_tier1_real(llm_client, &run_id, situation, &now, memory_ctx).await {
            Ok((fp, audits)) => {
                memory_audits.extend(audits);
                fp
            }
            Err(e) => {
                if allow_keyword_fallback {
                    tracing::warn!(run_id = %run_id, error = %e, "Tier 1 LLM classification failed, falling back to keyword");
                    keyword_fallback_fingerprint(&run_id, situation, &now)
                } else {
                    return Err(GameTheoryError::Tier1Execution {
                        message: e.to_string(),
                    });
                }
            }
        }
    } else if allow_keyword_fallback {
        keyword_fallback_fingerprint(&run_id, situation, &now)
    } else {
        return Err(GameTheoryError::LlmUnavailable {
            operation: "gametheory Tier 1 classification".to_string(),
        });
    };

    // Persist fingerprint
    let fingerprint_json =
        serde_json::to_string(&fingerprint).map_err(|e| GameTheoryError::FingerprintParse {
            message: e.to_string(),
        })?;

    insert_gt_fingerprint(
        db,
        &run_id,
        &fingerprint_json,
        &fingerprint.primary_family,
        &now,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    // Update run status to completed
    let completed_at = Utc::now().to_rfc3339();
    update_gt_run_status(
        db,
        &run_id,
        situation,
        &now,
        &completed_at,
        "completed",
        0.0,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    Ok((fingerprint, memory_audits))
}

/// Run the full Phase 4 pipeline: classify → route → specialist spec → final report.
///
/// Full specialist execution requires a real LLM provider. Classify-only may use
/// the labelled keyword fallback, but full runs must not persist fabricated
/// specialist outputs.
///
/// Persists all intermediate artefacts to Cozo.
pub async fn run_full_pipeline(
    db: &DbInstance,
    situation: &str,
    spec_path: Option<&Path>,
    llm: Option<&dyn LlmClient>,
) -> Result<FullPipelineResult, GameTheoryError> {
    run_full_pipeline_with_memory(
        db,
        situation,
        spec_path,
        llm,
        GameTheoryMemoryContext::default(),
    )
    .await
}

pub async fn run_full_pipeline_with_memory(
    db: &DbInstance,
    situation: &str,
    spec_path: Option<&Path>,
    llm: Option<&dyn LlmClient>,
    memory_ctx: GameTheoryMemoryContext,
) -> Result<FullPipelineResult, GameTheoryError> {
    run_full_pipeline_with_options(
        db,
        situation,
        spec_path,
        llm,
        memory_ctx,
        GameTheoryRunOptions::default(),
    )
    .await
}

pub async fn run_full_pipeline_with_options(
    db: &DbInstance,
    situation: &str,
    spec_path: Option<&Path>,
    llm: Option<&dyn LlmClient>,
    memory_ctx: GameTheoryMemoryContext,
    options: GameTheoryRunOptions,
) -> Result<FullPipelineResult, GameTheoryError> {
    let llm_client = require_llm(llm, "gametheory full pipeline")?;
    let kb_context = load_kb_run_context(db, options.kb_pack_id.as_deref()).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;
    let analysis_situation = situation_with_kb_context(situation, &kb_context);

    // 1. Tier 1 classification
    let (fingerprint, mut memory_recall) = classify_internal(
        db,
        &analysis_situation,
        Some(llm_client),
        &memory_ctx,
        false,
    )
    .await?;
    update_gt_run_status(
        db,
        &fingerprint.run_id,
        &analysis_situation,
        &fingerprint.created_at,
        "",
        "InProgress",
        0.0,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    if kb_context.pack_id.is_some() {
        persist_run_checkpoint(
            db,
            &fingerprint.run_id,
            "stage:kb-context",
            "stage",
            "completed",
            serde_json::json!({
                "kb": kb_context.pack_id,
                "documents": kb_context.document_count,
                "chunks": kb_context.chunk_count,
                "warning": kb_context.warning,
            }),
        )
        .map_err(|e| GameTheoryError::Storage {
            message: e.to_string(),
        })?;
    }
    persist_run_checkpoint(
        db,
        &fingerprint.run_id,
        "stage:tier1",
        "stage",
        "completed",
        serde_json::json!({"fingerprint_id": fingerprint.run_id}),
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    // 2. Resolve and load routing spec
    let resolved_path = resolve_spec_path(spec_path)?;
    let spec = load_spec(&resolved_path)?;

    // 3. Evaluate routing
    let now = Utc::now().to_rfc3339();
    let mut routing_decision = evaluate_routing(&spec, &fingerprint, &fingerprint.run_id, &now)?;

    // Tier 11 agents are high-impact intervention/mechanism specialists and
    // require both a CLI request and policy approval before they enter a run.
    let dep_map = build_dependency_map(&spec);
    apply_policy_gates_to_routing(&mut routing_decision, &dep_map, options.enable_tier11);

    // 4. Persist routing decision
    persist_routing_decision(db, &routing_decision).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    persist_run_checkpoint(
        db,
        &routing_decision.run_id,
        "stage:routing",
        "stage",
        "completed",
        serde_json::json!({"enabled": routing_decision.enabled_specialists.len()}),
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    // 5. Build dependency map from spec agent entries

    // 6. Build specialist DAG spec
    let _pipeline_spec =
        build_specialist_spec(&routing_decision, &dep_map, &spec, &analysis_situation);

    // 7. Execute specialist DAG with the configured LLM. Individual specialist
    // failures are isolated inside the outcome; a provider-level failure is
    // returned rather than replaced with fake output.
    let specialist_outcome = execute_specialists_real_with_options(
        llm_client,
        &routing_decision,
        &fingerprint,
        &analysis_situation,
        &memory_ctx,
        &options,
    )
    .await?;
    memory_recall.extend(specialist_outcome.memory_audits.clone());

    // 8. Persist specialist outputs
    persist_specialist_outputs(
        db,
        &routing_decision.run_id,
        &specialist_outcome.outputs,
        &specialist_outcome.costs_usd,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    persist_tier_checkpoints(db, &routing_decision.run_id, &specialist_outcome.outputs).map_err(
        |e| GameTheoryError::Storage {
            message: e.to_string(),
        },
    )?;

    // 9. Run quality checks
    let mut quality_results: HashMap<String, Vec<quality::QualityCheck>> = HashMap::new();
    for (key, output) in &specialist_outcome.outputs {
        let checks = quality::run_advisory_gates(key, output);
        quality_results.insert(key.clone(), checks);
    }
    persist_quality_checks(db, &routing_decision.run_id, &quality_results).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;

    // 10. Final stage assembly
    let final_result = final_stage::assemble_report(
        &specialist_outcome.outputs,
        &quality_results,
        options.style_profile_id.as_deref(),
    );
    let report = if specialist_outcome.budget_exceeded {
        format!("[BUDGET-EXCEEDED]\n\n{}", final_result.report)
    } else {
        final_result.report.clone()
    };

    // 11. Persist sections and final report
    persist_sections(db, &routing_decision.run_id, &final_result.sections).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;
    persist_final_report(
        db,
        &routing_decision.run_id,
        &report,
        specialist_outcome.total_cost_usd,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    persist_provenance_edges_for_run(
        db,
        &routing_decision.run_id,
        specialist_outcome.outputs.keys(),
        &final_result.sections,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    persist_run_checkpoint(
        db,
        &routing_decision.run_id,
        "stage:final-report",
        "stage",
        "completed",
        serde_json::json!({"words": report.split_whitespace().count()}),
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    let status = if specialist_outcome.budget_exceeded {
        "BudgetExceeded".to_string()
    } else if specialist_outcome.failed.is_empty() {
        "completed".to_string()
    } else {
        "partial".to_string()
    };
    let completed_at = Utc::now().to_rfc3339();
    update_gt_run_status(
        db,
        &routing_decision.run_id,
        situation,
        &fingerprint.created_at,
        &completed_at,
        &status,
        specialist_outcome.total_cost_usd,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    Ok(FullPipelineResult {
        run_id: routing_decision.run_id.clone(),
        fingerprint,
        routing_decision,
        report,
        specialist_count: specialist_outcome.outputs.len(),
        failed_specialists: specialist_outcome.failed,
        memory_recall,
        total_cost_usd: specialist_outcome.total_cost_usd,
        specialist_costs_usd: specialist_outcome.costs_usd,
        tier_costs_usd: specialist_outcome.tier_costs_usd,
        max_observed_concurrent: specialist_outcome.max_observed_concurrent,
        status,
    })
}

#[cfg(test)]
mod tests;
