use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use cozo::DbInstance;

use super::super::errors::GameTheoryError;
use super::super::final_stage;
use super::super::registry::GAMETHEORY_AGENTS;
use super::super::routing::{RoutingDecision, evaluate_routing, load_spec, resolve_spec_path};
use super::super::schema::ensure_gametheory_schema;
use super::loaders::{
    load_completed_specialist_keys, load_completed_specialist_outputs, load_run_situation,
    load_run_state, load_specialist_cost_total, load_stored_fingerprint, load_stored_routing,
    summarize_output,
};
use super::persistence::{
    persist_final_report, persist_provenance_edges_for_run, persist_routing_decision,
    persist_run_checkpoint, persist_sections, persist_specialist_failure,
    persist_specialist_outputs, persist_tier_checkpoints, update_gt_run_status,
};
use super::policy::{apply_policy_gates_to_routing, build_dependency_map};
use super::specialists::execute_specialists_real_with_options;
use super::types::{
    InProgressRun, ReplaySpecialistResult, ResumeRunResult, SpecialistExecutionOutcome,
};
use super::{GameTheoryMemoryContext, GameTheoryRunOptions, require_llm};
use crate::learning::integration::LearningIntegration;
use crate::runner::LlmClient;

/// Re-evaluate routing from the stored Tier 1 fingerprint and persist the
/// refreshed routing decision for inspection/replay auditability.
pub fn replay_routing_from_stored_fingerprint(
    db: &DbInstance,
    run_id: &str,
    spec_path: Option<&Path>,
) -> Result<RoutingDecision, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let fingerprint = load_stored_fingerprint(db, run_id)?;
    let resolved_path = resolve_spec_path(spec_path)?;
    let spec = load_spec(&resolved_path)?;
    let now = Utc::now().to_rfc3339();
    let routing_decision = evaluate_routing(&spec, &fingerprint, run_id, &now)?;
    persist_routing_decision(db, &routing_decision).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    Ok(routing_decision)
}

/// Re-run exactly one specialist using a stored Tier 1 fingerprint.
///
/// This is the CLI-backed source-of-truth implementation for
/// `archon gametheory replay --rerun-specialist <key>`.
pub async fn replay_single_specialist(
    db: &DbInstance,
    run_id: &str,
    agent_key: &str,
    llm: Option<&dyn LlmClient>,
    memory_ctx: GameTheoryMemoryContext,
    options: GameTheoryRunOptions,
) -> Result<ReplaySpecialistResult, GameTheoryError> {
    replay_single_specialist_with_learning(db, run_id, agent_key, llm, memory_ctx, options, None)
        .await
}

pub async fn replay_single_specialist_with_learning(
    db: &DbInstance,
    run_id: &str,
    agent_key: &str,
    llm: Option<&dyn LlmClient>,
    memory_ctx: GameTheoryMemoryContext,
    options: GameTheoryRunOptions,
    learning: Option<&mut LearningIntegration>,
) -> Result<ReplaySpecialistResult, GameTheoryError> {
    if !GAMETHEORY_AGENTS.iter().any(|agent| agent.key == agent_key) {
        return Err(GameTheoryError::AgentNotFound {
            key: agent_key.to_string(),
        });
    }

    let situation = load_run_situation(db, run_id)?;
    let fingerprint = load_stored_fingerprint(db, run_id)?;
    let now = Utc::now().to_rfc3339();
    let routing = RoutingDecision {
        run_id: run_id.to_string(),
        fingerprint_id: fingerprint.run_id.clone(),
        enabled_specialists: vec![agent_key.to_string()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: now,
    };

    let llm_client = require_llm(llm, "gametheory replay --rerun-specialist")?;
    let outcome = execute_specialists_real_with_options(
        llm_client,
        &routing,
        &fingerprint,
        &situation,
        &memory_ctx,
        &options,
        learning,
    )
    .await?;

    if let Some(output) = outcome.outputs.get(agent_key) {
        persist_specialist_outputs(db, run_id, &outcome.outputs, &outcome.costs_usd).map_err(
            |e| GameTheoryError::Storage {
                message: e.to_string(),
            },
        )?;
        let cost = outcome.costs_usd.get(agent_key).copied().unwrap_or(0.0);
        return Ok(ReplaySpecialistResult {
            run_id: run_id.to_string(),
            agent_key: agent_key.to_string(),
            status: "completed".to_string(),
            output_summary: summarize_output(output),
            cost_usd: cost,
            memory_recall: outcome.memory_audits,
        });
    }

    let message = outcome
        .failed
        .iter()
        .find(|(key, _)| key == agent_key)
        .map(|(_, err)| err.clone())
        .unwrap_or_else(|| "budget cap prevented specialist replay".to_string());
    persist_specialist_failure(db, run_id, agent_key, &message).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;
    Ok(ReplaySpecialistResult {
        run_id: run_id.to_string(),
        agent_key: agent_key.to_string(),
        status: "failed".to_string(),
        output_summary: summarize_output(&message),
        cost_usd: 0.0,
        memory_recall: outcome.memory_audits,
    })
}

pub fn list_in_progress_runs(db: &DbInstance) -> Result<Vec<InProgressRun>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let rows = db
        .run_script(
            "?[run_id, situation, started_at] := *gt_runs{run_id, situation, started_at, completed_at, status}, status = 'InProgress'",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query in-progress gt_runs failed: {e}"),
        })?;
    Ok(rows
        .rows
        .iter()
        .map(|row| InProgressRun {
            run_id: row[0].get_str().unwrap_or("").to_string(),
            situation: row[1].get_str().unwrap_or("").to_string(),
            started_at: row[2].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

pub async fn resume_run_from_checkpoint(
    db: &DbInstance,
    run_id: &str,
    spec_path: Option<&Path>,
    llm: Option<&dyn LlmClient>,
    memory_ctx: GameTheoryMemoryContext,
    options: GameTheoryRunOptions,
) -> Result<ResumeRunResult, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let run_state = load_run_state(db, run_id)?;
    if run_state.status != "InProgress" {
        return Err(GameTheoryError::Validation {
            message: format!(
                "run {run_id} has status '{}' and is not resumable",
                run_state.status
            ),
        });
    }
    let situation = run_state.situation.clone();
    let fingerprint = load_stored_fingerprint(db, run_id)?;
    let mut routing = match load_stored_routing(db, run_id)? {
        Some(routing) => routing,
        None => {
            let resolved_path = resolve_spec_path(spec_path)?;
            let spec = load_spec(&resolved_path)?;
            let mut routing =
                evaluate_routing(&spec, &fingerprint, run_id, &Utc::now().to_rfc3339())?;
            let dep_map = build_dependency_map(&spec);
            apply_policy_gates_to_routing(&mut routing, &dep_map, options.enable_tier11);
            persist_routing_decision(db, &routing).map_err(|e| GameTheoryError::Storage {
                message: e.to_string(),
            })?;
            persist_run_checkpoint(
                db,
                run_id,
                "stage:routing",
                "stage",
                "completed",
                serde_json::json!({"enabled": routing.enabled_specialists.len(), "recovered": true}),
            )
            .map_err(|e| GameTheoryError::Storage {
                message: e.to_string(),
            })?;
            routing
        }
    };
    let completed = load_completed_specialist_keys(db, run_id)?;
    let skipped_completed = routing
        .enabled_specialists
        .iter()
        .filter(|key| completed.contains(*key))
        .count();
    routing
        .enabled_specialists
        .retain(|key| !completed.contains(key));

    persist_run_checkpoint(
        db,
        run_id,
        "stage:resume-start",
        "stage",
        "completed",
        serde_json::json!({"remaining": routing.enabled_specialists.len()}),
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    let outcome = if routing.enabled_specialists.is_empty() {
        SpecialistExecutionOutcome::default()
    } else {
        let llm_client = require_llm(llm, "gametheory resume")?;
        execute_specialists_real_with_options(
            llm_client,
            &routing,
            &fingerprint,
            &situation,
            &memory_ctx,
            &options,
            None,
        )
        .await?
    };
    persist_specialist_outputs(db, run_id, &outcome.outputs, &outcome.costs_usd).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;
    for (agent_key, message) in &outcome.failed {
        persist_specialist_failure(db, run_id, agent_key, message).map_err(|e| {
            GameTheoryError::Storage {
                message: e.to_string(),
            }
        })?;
    }
    persist_tier_checkpoints(db, run_id, &outcome.outputs).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;

    let all_outputs = load_completed_specialist_outputs(db, run_id)?;
    let quality_results = HashMap::new();
    let final_result = final_stage::assemble_report(
        &all_outputs,
        &quality_results,
        options.style_profile_id.as_deref(),
    );
    persist_sections(db, run_id, &final_result.sections).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let total_cost_usd = load_specialist_cost_total(db, run_id)?;
    let status = if outcome.budget_exceeded {
        "BudgetExceeded"
    } else if outcome.failed.is_empty() {
        "completed"
    } else {
        "partial"
    };
    persist_final_report(db, run_id, &final_result.report, total_cost_usd).map_err(|e| {
        GameTheoryError::Storage {
            message: e.to_string(),
        }
    })?;
    persist_provenance_edges_for_run(db, run_id, all_outputs.keys(), &final_result.sections)
        .map_err(|e| GameTheoryError::Storage {
            message: e.to_string(),
        })?;
    update_gt_run_status(
        db,
        run_id,
        &situation,
        &run_state.started_at,
        &Utc::now().to_rfc3339(),
        status,
        total_cost_usd,
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    persist_run_checkpoint(
        db,
        run_id,
        "stage:resume-complete",
        "stage",
        status,
        serde_json::json!({
            "resumed": outcome.outputs.len(),
            "failed": outcome.failed.len(),
            "total_cost_usd": total_cost_usd,
        }),
    )
    .map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;

    Ok(ResumeRunResult {
        run_id: run_id.to_string(),
        resumed_specialists: outcome.outputs.len(),
        skipped_completed_specialists: skipped_completed,
        failed_specialists: outcome.failed.len(),
        status: status.to_string(),
        total_cost_usd,
        report_words: final_result.report.split_whitespace().count(),
    })
}
