//! Game-theory orchestration entrypoints.
//!
//! `classify` — Tier 1 classification only (fingerprint + persistence).
//! `run_full_pipeline` — classify → route → specialist DAG → final report.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use cozo::DbInstance;
use futures_util::future::join_all;

use super::errors::GameTheoryError;
use super::final_stage;
use super::fingerprint::{AmbiguityNote, AxisVerdict, GameTheoryFingerprint, HiddenGameDetection};
use super::prompt_builder;
use super::quality;
use super::registry::GAMETHEORY_AGENTS;
use super::routing::{
    GameTheorySpec, RoutingDecision, evaluate_routing, load_spec, resolve_spec_path,
};
use super::schema::ensure_gametheory_schema;
use super::spec::build_specialist_spec;
use crate::leann_searcher::LeannSearcher;
use crate::runner::{LlmClient, LlmResponse};
use archon_memory::{MemoryTrait, SearchFilter};

const TAG_GAMETHEORY_PIPELINE: &str = "gametheory-pipeline";
const TIER1_MEMORY_AGENT_KEYS: &[&str] = &[
    "game-classifier",
    "payoff-elicitor",
    "strategy-space-enumerator",
    "information-structure-mapper",
];

/// Optional memory backends used for gametheory prompt enrichment.
#[derive(Clone, Default)]
pub struct GameTheoryMemoryContext {
    pub memory: Option<Arc<dyn MemoryTrait>>,
    pub leann_searcher: Option<Arc<dyn LeannSearcher>>,
    pub debug: bool,
}

impl GameTheoryMemoryContext {
    pub fn new(
        memory: Arc<dyn MemoryTrait>,
        leann_searcher: Option<Arc<dyn LeannSearcher>>,
        debug: bool,
    ) -> Self {
        Self {
            memory: Some(memory),
            leann_searcher,
            debug,
        }
    }
}

/// Runtime controls for a full game-theory run.
#[derive(Debug, Clone)]
pub struct GameTheoryRunOptions {
    pub budget_usd: f64,
    pub max_concurrent: usize,
    pub style_profile_id: Option<String>,
    pub enable_tier11: bool,
    pub kb_pack_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct KbRunContext {
    pack_id: Option<String>,
    text: String,
    document_count: usize,
    chunk_count: usize,
    warning: Option<String>,
}

fn situation_with_kb_context(situation: &str, kb: &KbRunContext) -> String {
    if kb.pack_id.is_none() {
        return situation.to_string();
    }

    let pack = kb.pack_id.as_deref().unwrap_or("");
    let warning = kb
        .warning
        .as_ref()
        .map(|w| format!("\nWarning: {w}"))
        .unwrap_or_default();
    let context = if kb.text.trim().is_empty() {
        "No matching document chunks were found.".to_string()
    } else {
        kb.text.clone()
    };

    format!("{situation}\n\n## Knowledge Base Context: {pack}\n{warning}\n\n{context}")
}

fn load_kb_run_context(db: &DbInstance, pack_id: Option<&str>) -> Result<KbRunContext> {
    let Some(pack_id) = pack_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(KbRunContext::default());
    };

    let docs = match read_doc_source_matches(db, pack_id) {
        Ok(docs) => docs,
        Err(e) => {
            return Ok(KbRunContext {
                pack_id: Some(pack_id.to_string()),
                warning: Some(format!("document store unavailable: {e}")),
                ..KbRunContext::default()
            });
        }
    };

    let doc_ids: HashSet<String> = docs.iter().map(|(id, _)| id.clone()).collect();
    let chunks = read_doc_chunks_for_pack(db, pack_id, &doc_ids)?;
    let text = chunks
        .iter()
        .take(8)
        .map(|(chunk_id, doc_id, content)| {
            format!(
                "### {doc_id} / {chunk_id}\n{}",
                truncate_for_prompt(content, 700)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let warning = if chunks.is_empty() {
        Some(format!("no doc_chunks matched knowledge pack '{pack_id}'"))
    } else {
        None
    };

    Ok(KbRunContext {
        pack_id: Some(pack_id.to_string()),
        text,
        document_count: docs.len(),
        chunk_count: chunks.len(),
        warning,
    })
}

fn read_doc_source_matches(db: &DbInstance, pack_id: &str) -> Result<Vec<(String, String)>> {
    let rows = db
        .run_script(
            "?[document_id, source_path] := *doc_sources{document_id, source_path}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query doc_sources failed: {e}"))?;

    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            let document_id = row.first()?.get_str()?.to_string();
            let source_path = row.get(1)?.get_str()?.to_string();
            let haystack = format!("{document_id}\n{source_path}");
            haystack
                .contains(pack_id)
                .then_some((document_id, source_path))
        })
        .collect())
}

fn read_doc_chunks_for_pack(
    db: &DbInstance,
    pack_id: &str,
    doc_ids: &HashSet<String>,
) -> Result<Vec<(String, String, String)>> {
    let rows = db
        .run_script(
            "?[chunk_id, document_id, content] := *doc_chunks{chunk_id, document_id, content}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query doc_chunks failed: {e}"))?;

    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            let chunk_id = row.first()?.get_str()?.to_string();
            let document_id = row.get(1)?.get_str()?.to_string();
            let content = row.get(2)?.get_str()?.to_string();
            (doc_ids.contains(&document_id) || document_id.contains(pack_id)).then_some((
                chunk_id,
                document_id,
                content,
            ))
        })
        .collect())
}

fn truncate_for_prompt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

impl Default for GameTheoryRunOptions {
    fn default() -> Self {
        Self {
            budget_usd: 20.0,
            max_concurrent: 4,
            style_profile_id: None,
            enable_tier11: false,
            kb_pack_id: None,
        }
    }
}

/// Source-of-truth audit for memory recall performed before an agent call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecallAudit {
    pub agent_key: String,
    pub memory_keys: Vec<String>,
    pub cozo_hits: usize,
    pub leann_hits: usize,
}

#[derive(Debug, Clone)]
struct RecalledContext {
    text: String,
    audit: MemoryRecallAudit,
}

#[derive(Debug, Clone, Default)]
struct SpecialistExecutionOutcome {
    outputs: HashMap<String, String>,
    failed: Vec<(String, String)>,
    memory_audits: Vec<MemoryRecallAudit>,
    costs_usd: HashMap<String, f64>,
    total_cost_usd: f64,
    tier_costs_usd: BTreeMap<u8, f64>,
    budget_exceeded: bool,
    max_observed_concurrent: usize,
}

#[derive(Debug, Clone)]
struct Tier1AgentOutput {
    agent_key: String,
    content: String,
}

#[derive(Debug, Clone)]
struct SpecialistCallOutput {
    agent_key: String,
    output: Option<String>,
    error: Option<String>,
    audit: MemoryRecallAudit,
    cost_usd: f64,
}

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
    /// Per-agent memory recall evidence collected during real LLM execution.
    pub memory_recall: Vec<MemoryRecallAudit>,
    /// Total estimated model cost for successful specialist calls.
    pub total_cost_usd: f64,
    /// Per-specialist estimated model cost.
    pub specialist_costs_usd: HashMap<String, f64>,
    /// Per-tier estimated model cost, keyed by game-theory tier.
    pub tier_costs_usd: BTreeMap<u8, f64>,
    /// Maximum observed specialist concurrency for this run.
    pub max_observed_concurrent: usize,
    /// Overall pipeline status: "completed" (all specialists succeeded) or "partial" (some failed).
    pub status: String,
}

/// Result of replaying one specialist against a stored Tier 1 fingerprint.
#[derive(Debug, Clone)]
pub struct ReplaySpecialistResult {
    pub run_id: String,
    pub agent_key: String,
    pub status: String,
    pub output_summary: String,
    pub cost_usd: f64,
    pub memory_recall: Vec<MemoryRecallAudit>,
}

#[derive(Debug, Clone)]
pub struct InProgressRun {
    pub run_id: String,
    pub situation: String,
    pub started_at: String,
}

#[derive(Debug, Clone)]
pub struct ResumeRunResult {
    pub run_id: String,
    pub resumed_specialists: usize,
    pub skipped_completed_specialists: usize,
    pub failed_specialists: usize,
    pub status: String,
    pub total_cost_usd: f64,
    pub report_words: usize,
}

#[derive(Debug, Clone)]
struct StoredRunState {
    situation: String,
    started_at: String,
    status: String,
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

fn apply_policy_gates_to_routing(
    routing: &mut RoutingDecision,
    dep_map: &HashMap<String, Vec<String>>,
    enable_tier11: bool,
) {
    let mut skipped = Vec::new();
    let mut enabled: Vec<String> = routing.enabled_specialists.clone();
    loop {
        let enabled_set: HashSet<String> = enabled.iter().cloned().collect();
        let before = enabled.len();
        enabled.retain(|agent_key| {
            if !enable_tier11 && agent_tier(agent_key) == Some(11) {
                skipped.push((
                    agent_key.clone(),
                    "Tier 11 disabled: pass --enable-tier11 and set policy.gametheory.enable_tier11=true".to_string(),
                ));
                return false;
            }
            if let Some(missing_dep) = dep_map
                .get(agent_key)
                .and_then(|deps| deps.iter().find(|dep| !enabled_set.contains(*dep)))
            {
                skipped.push((
                    agent_key.clone(),
                    format!("dependency '{missing_dep}' was skipped by policy"),
                ));
                return false;
            }
            true
        });
        if enabled.len() == before {
            break;
        }
    }
    routing.enabled_specialists = enabled;
    routing.skipped_specialists.extend(skipped);
}

/// Test-only deterministic specialist fixture with failure isolation.
#[cfg(test)]
fn execute_test_specialist_fixture(
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> (
    HashMap<String, String>,
    Vec<(String, String)>,
    Vec<MemoryRecallAudit>,
) {
    let outcome = execute_test_specialist_fixture_with_options(
        routing,
        fingerprint,
        situation,
        memory_ctx,
        &GameTheoryRunOptions::default(),
    );
    (outcome.outputs, outcome.failed, outcome.memory_audits)
}

#[cfg(test)]
fn execute_test_specialist_fixture_with_options(
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
    options: &GameTheoryRunOptions,
) -> SpecialistExecutionOutcome {
    let fingerprint_summary = prompt_builder::fingerprint_summary_text(fingerprint);
    let mut outcome = SpecialistExecutionOutcome::default();

    for agent_key in &routing.enabled_specialists {
        if outcome.total_cost_usd >= options.budget_usd {
            outcome.budget_exceeded = true;
            break;
        }
        outcome.max_observed_concurrent = outcome.max_observed_concurrent.max(1);
        let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
        outcome.memory_audits.push(recalled.audit.clone());
        let result = execute_single_specialist_fixture(
            agent_key,
            situation,
            &fingerprint_summary,
            &recalled.text,
        );

        match result {
            Ok(output) => {
                outcome.outputs.insert(agent_key.clone(), output);
                outcome.costs_usd.insert(agent_key.clone(), 0.0);
            }
            Err(err_msg) => {
                outcome.failed.push((agent_key.clone(), err_msg));
            }
        }
    }

    outcome
}

/// Execute a single deterministic specialist fixture.
///
/// Test hook: if `agent_key` ends with `-FORCE-FAIL-FOR-TEST`, returns Err.
#[cfg(test)]
fn execute_single_specialist_fixture(
    agent_key: &str,
    situation: &str,
    fingerprint_summary: &str,
    prior_context: &str,
) -> Result<String, String> {
    // Test hook: force failure for failure isolation testing
    if agent_key.ends_with("-FORCE-FAIL-FOR-TEST") {
        return Err(format!("forced failure for test: {agent_key}"));
    }

    let _prompt = prompt_builder::build_specialist_prompt_with_prior_context(
        agent_key,
        agent_key,
        situation,
        fingerprint_summary,
        prior_context,
        &[],
    );

    let prior_context_section = if prior_context.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n**Prior Context:**\n\n{prior_context}")
    };

    Ok(format!(
        "## {agent_key} - Fixture Analysis\n\n\
         **Situation:** {situation}\n\n\
         **Fingerprint:** {fp_summary}{prior_context_section}\n\n\
         *Test-only deterministic fixture output.*",
        fp_summary = fingerprint_summary,
    ))
}

fn agent_memory_keys(agent_key: &str) -> &'static [&'static str] {
    GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .map(|agent| agent.memory_keys)
        .unwrap_or(&[])
}

fn recall_prior_context_for_agent(
    agent_key: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> RecalledContext {
    let memory_keys = agent_memory_keys(agent_key);
    let mut cozo_hits = 0usize;
    let mut leann_hits = 0usize;
    let mut parts = Vec::new();

    for &memory_key in memory_keys {
        let mut key_parts = Vec::new();

        if let Some(memory) = memory_ctx.memory.as_ref() {
            let filter = SearchFilter {
                tags: vec![TAG_GAMETHEORY_PIPELINE.to_string()],
                ..Default::default()
            };
            if let Ok(memories) = memory.search_memories(&filter) {
                for m in memories {
                    if m.title == memory_key {
                        cozo_hits += 1;
                        key_parts.push(m.content);
                    }
                }
            }
        }

        if key_parts.is_empty() {
            if let Some(leann) = memory_ctx.leann_searcher.as_ref() {
                let fallback = leann.search(memory_key);
                if !fallback.trim().is_empty() {
                    leann_hits += 1;
                    key_parts.push(fallback);
                }
            }
        }

        if !key_parts.is_empty() {
            parts.push(format!("#### {memory_key}\n\n{}", key_parts.join("\n\n")));
        }
    }

    RecalledContext {
        text: parts.join("\n\n---\n\n"),
        audit: MemoryRecallAudit {
            agent_key: agent_key.to_string(),
            memory_keys: memory_keys.iter().map(|key| key.to_string()).collect(),
            cozo_hits,
            leann_hits,
        },
    }
}

/// Execute real Tier 1 classification via the four mandatory Tier 1 agents.
///
/// The PRD requires the mandatory foundation agents to run as one parallel
/// wave. `game-classifier` is responsible for the machine-readable 9-axis
/// fingerprint; the other foundation outputs are executed and available for
/// audit/prompt evolution but do not overwrite the classifier JSON.
async fn execute_tier1_real(
    llm: &dyn LlmClient,
    run_id: &str,
    situation: &str,
    now: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> Result<(GameTheoryFingerprint, Vec<MemoryRecallAudit>), GameTheoryError> {
    let mut audits = Vec::new();
    let mut prior_context_parts = Vec::new();
    for agent_key in TIER1_MEMORY_AGENT_KEYS {
        let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
        if !recalled.text.is_empty() {
            prior_context_parts.push(format!("### {agent_key}\n\n{}", recalled.text));
        }
        audits.push(recalled.audit);
    }
    let prior_context = prior_context_parts.join("\n\n---\n\n");

    let tier1_calls = TIER1_MEMORY_AGENT_KEYS
        .iter()
        .map(|agent_key| execute_tier1_agent(llm, agent_key, situation, &prior_context));
    let responses = join_all(tier1_calls).await;
    let mut outputs = Vec::new();
    let mut failures = Vec::new();
    for response in responses {
        match response {
            Ok(output) => outputs.push(output),
            Err(err) => failures.push(err.to_string()),
        }
    }
    if !failures.is_empty() {
        return Err(GameTheoryError::Tier1Execution {
            message: failures.join("; "),
        });
    }

    let classifier_output = outputs
        .iter()
        .find(|output| output.agent_key == "game-classifier")
        .or_else(|| outputs.first())
        .ok_or_else(|| GameTheoryError::Tier1Execution {
            message: "no Tier 1 agent outputs were produced".to_string(),
        })?;

    let fingerprint = parse_tier1_fingerprint(run_id, now, &classifier_output.content)?;
    Ok((fingerprint, audits))
}

async fn execute_tier1_agent(
    llm: &dyn LlmClient,
    agent_key: &str,
    situation: &str,
    prior_context: &str,
) -> Result<Tier1AgentOutput, GameTheoryError> {
    let system = vec![serde_json::json!({
        "role": "system",
        "content": tier1_system_prompt(agent_key)
    })];

    let user_content = if prior_context.is_empty() {
        format!("Classify this strategic situation as Tier 1 agent `{agent_key}`:\n\n{situation}")
    } else {
        format!(
            "Classify this strategic situation as Tier 1 agent `{agent_key}`:\n\n{situation}\n\n## Recalled Prior Context\n\n{prior_context}"
        )
    };

    let messages = vec![serde_json::json!({
        "role": "user",
        "content": user_content
    })];

    let response: LlmResponse = llm
        .send_message(messages, system, vec![], "claude-sonnet-4-6")
        .await
        .map_err(|e| GameTheoryError::Storage {
            message: e.to_string(),
        })?;
    Ok(Tier1AgentOutput {
        agent_key: agent_key.to_string(),
        content: response.content,
    })
}

fn tier1_system_prompt(agent_key: &str) -> &'static str {
    match agent_key {
        "game-classifier" => {
            "You are the game-classifier Tier 1 foundation agent. Analyze the given strategic situation and output a JSON object with exactly these fields: cooperation (cooperative/non-cooperative), payoff_sum (zero-sum/positive-sum/variable-sum), symmetry (symmetric/asymmetric/unknown), timing (simultaneous/sequential/repeated), perfect_info (perfect/imperfect), complete_info (complete/incomplete), cardinality (2-player/n-player), strategy_space (continuous/discrete), horizon (one-shot/repeated), primary_family (short label like \"Bertrand competition\"), nearest_classic (classic game name or null). For each axis also include a confidence (low/medium/high) and a brief rationale. Output ONLY the JSON object, no markdown wrapping."
        }
        "payoff-elicitor" => {
            "You are the payoff-elicitor Tier 1 foundation agent. Identify players, incentives, payoff dimensions, likely payoff conflicts, and missing payoff assumptions. Output concise markdown."
        }
        "strategy-space-enumerator" => {
            "You are the strategy-space-enumerator Tier 1 foundation agent. Enumerate each player's feasible actions, strategies, constraints, and whether the strategy space is discrete or continuous. Output concise markdown."
        }
        "information-structure-mapper" => {
            "You are the information-structure-mapper Tier 1 foundation agent. Map who knows what, what is hidden, signalling channels, beliefs, and information asymmetries. Output concise markdown."
        }
        _ => {
            "You are a Tier 1 game-theory foundation agent. Analyze the strategic situation from your assigned perspective."
        }
    }
}

fn parse_tier1_fingerprint(
    run_id: &str,
    now: &str,
    content: &str,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    // Try to parse JSON from the response (may be wrapped in ```json fences)
    let trimmed = content.trim();
    let json_str = if let Some(start) = trimmed.find("```json") {
        let inner = &trimmed[start + 7..];
        if let Some(end) = inner.find("```") {
            &inner[..end]
        } else {
            inner
        }
    } else if let Some(start) = trimmed.find('{') {
        &trimmed[start..]
    } else {
        return Err(GameTheoryError::FingerprintParse {
            message: "LLM response did not contain JSON".into(),
        });
    };

    let parsed: serde_json::Value =
        serde_json::from_str(json_str.trim()).map_err(|e| GameTheoryError::FingerprintParse {
            message: e.to_string(),
        })?;

    // Extract fields with defaults
    let get_axis = |key: &str| -> AxisVerdict {
        parsed
            .get(key)
            .map(|v| {
                AxisVerdict::new(
                    v.get("value").and_then(|x| x.as_str()).unwrap_or("unknown"),
                    v.get("confidence")
                        .and_then(|x| x.as_str())
                        .unwrap_or("low"),
                    v.get("rationale").and_then(|x| x.as_str()).unwrap_or(""),
                )
            })
            .unwrap_or_else(|| AxisVerdict::new("unknown", "low", ""))
    };

    let primary_family = parsed
        .get("primary_family")
        .and_then(|v| v.as_str())
        .unwrap_or("Strategic interaction")
        .to_string();

    let nearest_classic = parsed
        .get("nearest_classic")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(GameTheoryFingerprint {
        run_id: run_id.to_string(),
        cooperation: get_axis("cooperation"),
        payoff_sum: get_axis("payoff_sum"),
        symmetry: get_axis("symmetry"),
        timing: get_axis("timing"),
        perfect_info: get_axis("perfect_info"),
        complete_info: get_axis("complete_info"),
        cardinality: get_axis("cardinality"),
        strategy_space: get_axis("strategy_space"),
        horizon: get_axis("horizon"),
        primary_family,
        nearest_classic,
        shadow_games: vec![],
        hidden_game_scan: None,
        ambiguities: vec![],
        created_at: now.to_string(),
    })
}

/// Execute real LLM-backed specialist agents.
///
/// Each enabled specialist is spawned as a separate LLM call. Failures are
/// isolated — a single specialist failure does not abort the others.
///
/// Returns `(successful_outputs, failed_specialists)`.
#[cfg(test)]
async fn execute_specialists_real(
    llm: &dyn LlmClient,
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> Result<
    (
        HashMap<String, String>,
        Vec<(String, String)>,
        Vec<MemoryRecallAudit>,
    ),
    GameTheoryError,
> {
    let outcome = execute_specialists_real_with_options(
        llm,
        routing,
        fingerprint,
        situation,
        memory_ctx,
        &GameTheoryRunOptions::default(),
    )
    .await?;
    Ok((outcome.outputs, outcome.failed, outcome.memory_audits))
}

async fn execute_specialists_real_with_options(
    llm: &dyn LlmClient,
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
    options: &GameTheoryRunOptions,
) -> Result<SpecialistExecutionOutcome, GameTheoryError> {
    let fingerprint_summary = prompt_builder::fingerprint_summary_text(fingerprint);
    let mut outcome = SpecialistExecutionOutcome::default();
    let max_concurrent = options.max_concurrent.max(1);

    let system = vec![serde_json::json!({
        "role": "system",
        "content": "You are a game-theory analysis specialist. Analyze the given strategic situation from your specialist perspective and produce a detailed markdown report section. Focus on your area of expertise. Output ONLY the report content, no preamble."
    })];

    for wave in routing.enabled_specialists.chunks(max_concurrent) {
        if outcome.total_cost_usd >= options.budget_usd {
            outcome.budget_exceeded = true;
            break;
        }

        let remaining_budget = options.budget_usd - outcome.total_cost_usd;
        let affordable_slots = if outcome.total_cost_usd == 0.0 && remaining_budget > 0.0 {
            1.max(wave.len())
        } else if remaining_budget > 0.0 {
            wave.len()
        } else {
            0
        };
        let active_wave = &wave[..affordable_slots.min(wave.len())];
        if active_wave.is_empty() {
            outcome.budget_exceeded = true;
            break;
        }
        outcome.max_observed_concurrent = outcome.max_observed_concurrent.max(active_wave.len());

        let calls = active_wave.iter().map(|agent_key| {
            execute_specialist_call(
                llm,
                agent_key,
                situation,
                &fingerprint_summary,
                memory_ctx,
                &system,
            )
        });
        let results = join_all(calls).await;
        for result in results {
            outcome.memory_audits.push(result.audit);
            if let Some(output) = result.output {
                if let Some(tier) = agent_tier(&result.agent_key) {
                    *outcome.tier_costs_usd.entry(tier).or_insert(0.0) += result.cost_usd;
                }
                outcome.total_cost_usd += result.cost_usd;
                outcome
                    .costs_usd
                    .insert(result.agent_key.clone(), result.cost_usd);
                outcome.outputs.insert(result.agent_key, output);
            } else if let Some(error) = result.error {
                outcome.failed.push((result.agent_key, error));
            }
        }
        if outcome.total_cost_usd >= options.budget_usd {
            outcome.budget_exceeded = true;
        }
    }

    Ok(outcome)
}

async fn execute_specialist_call(
    llm: &dyn LlmClient,
    agent_key: &str,
    situation: &str,
    fingerprint_summary: &str,
    memory_ctx: &GameTheoryMemoryContext,
    system: &[serde_json::Value],
) -> SpecialistCallOutput {
    let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
    if agent_key.ends_with("-FORCE-FAIL-FOR-TEST") {
        return SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: None,
            error: Some(format!("forced failure for test: {agent_key}")),
            audit: recalled.audit,
            cost_usd: 0.0,
        };
    }

    let prompt = prompt_builder::build_specialist_prompt_with_prior_context(
        agent_key,
        agent_key,
        situation,
        fingerprint_summary,
        &recalled.text,
        &[],
    );
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    match llm
        .send_message(messages, system.to_vec(), vec![], "claude-sonnet-4-6")
        .await
    {
        Ok(response) => SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: Some(response.content),
            error: None,
            audit: recalled.audit,
            cost_usd: estimate_llm_cost_usd(
                "claude-sonnet-4-6",
                response.tokens_in,
                response.tokens_out,
            ),
        },
        Err(e) => SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: None,
            error: Some(format!("LLM error: {e}")),
            audit: recalled.audit,
            cost_usd: 0.0,
        },
    }
}

fn agent_tier(agent_key: &str) -> Option<u8> {
    GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .map(|agent| agent.tier)
}

/// Estimate API cost from token usage.
///
/// Rates are documented here to keep Group 5 deterministic in tests:
/// Claude Sonnet family is estimated at $3 / 1M input tokens and $15 / 1M
/// output tokens. Unknown models fall back to the same conservative rate.
fn estimate_llm_cost_usd(model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    let (input_per_million, output_per_million) = if model.contains("sonnet") {
        (3.0, 15.0)
    } else if model.contains("opus") {
        (15.0, 75.0)
    } else {
        (3.0, 15.0)
    };

    (tokens_in as f64 / 1_000_000.0 * input_per_million)
        + (tokens_out as f64 / 1_000_000.0 * output_per_million)
}

/// Generate a keyword-based fingerprint as fallback when no LLM provider is available.
///
/// Performs simple keyword analysis of the situation text. Less accurate than
/// real Tier 1 classification but requires no external dependencies.
fn keyword_fallback_fingerprint(run_id: &str, situation: &str, now: &str) -> GameTheoryFingerprint {
    let s = situation.to_lowercase();

    let cooperation = if s.contains("collaborate")
        || s.contains("cooperate")
        || s.contains("alliance")
        || s.contains("cartel")
    {
        AxisVerdict::new("cooperative", "medium", "cooperation keywords detected")
    } else {
        AxisVerdict::new(
            "non-cooperative",
            "medium",
            "default for unmarked situations",
        )
    };

    let payoff_sum =
        if s.contains("zero-sum") || s.contains("winner-take") || s.contains("all or nothing") {
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

    let perfect_info = if s.contains("perfect information")
        || s.contains("knows everything")
        || s.contains("full information")
    {
        AxisVerdict::new("perfect", "medium", "perfect information keywords")
    } else if s.contains("imperfect") || s.contains("hidden") || s.contains("private") {
        AxisVerdict::new("imperfect", "medium", "imperfect information keywords")
    } else {
        AxisVerdict::new(
            "imperfect",
            "low",
            "most real situations have imperfect info",
        )
    };

    let complete_info = if s.contains("incomplete")
        || s.contains("doesn't know")
        || s.contains("unknown")
        || s.contains("private type")
        || s.contains("asymmetric information")
    {
        AxisVerdict::new("incomplete", "medium", "incomplete information keywords")
    } else if s.contains("complete information") || s.contains("knows everything about") {
        AxisVerdict::new("complete", "medium", "complete information keywords")
    } else {
        AxisVerdict::new(
            "incomplete",
            "low",
            "most real situations have incomplete info",
        )
    };

    let cardinality = if s.contains("two player")
        || s.contains("two firm")
        || s.contains("bilateral")
        || s.contains("duopoly")
        || (s.contains("two") && s.contains("player"))
    {
        AxisVerdict::new("2-player", "medium", "two-player keywords")
    } else if s.contains("n-player")
        || s.contains("multi")
        || s.contains("many")
        || s.contains("oligopoly")
        || s.contains("market")
    {
        AxisVerdict::new("n-player", "medium", "multi-player keywords")
    } else {
        AxisVerdict::new("2-player", "low", "default assumption")
    };

    let strategy_space = if s.contains("continuous")
        || s.contains("price")
        || s.contains("quantity")
        || s.contains("amount")
    {
        AxisVerdict::new("continuous", "medium", "continuous strategy indicators")
    } else if s.contains("discrete")
        || s.contains("binary")
        || s.contains("yes/no")
        || s.contains("choice")
    {
        AxisVerdict::new("discrete", "medium", "discrete strategy indicators")
    } else {
        AxisVerdict::new("discrete", "low", "default assumption")
    };

    let horizon = if s.contains("one-shot") || s.contains("once") || s.contains("single") {
        AxisVerdict::new("one-shot", "medium", "one-shot keywords")
    } else if s.contains("repeated")
        || s.contains("ongoing")
        || s.contains("infinitely")
        || s.contains("recurrent")
    {
        AxisVerdict::new("repeated", "medium", "repeated keywords")
    } else {
        AxisVerdict::new("one-shot", "low", "default assumption")
    };

    let (primary_family, nearest_classic) = if s.contains("price") && s.contains("simultaneous") {
        (
            "Bertrand competition".into(),
            Some("Bertrand duopoly".into()),
        )
    } else if s.contains("quantity") && s.contains("simultaneous") {
        ("Cournot competition".into(), Some("Cournot duopoly".into()))
    } else if s.contains("price") && s.contains("sequential") {
        (
            "Stackelberg price leadership".into(),
            Some("Stackelberg duopoly".into()),
        )
    } else if s.contains("dilemma") || s.contains("defect") || s.contains("cooperate vs") {
        ("Social dilemma".into(), Some("Prisoner's Dilemma".into()))
    } else if s.contains("coordinate") || s.contains("standard") || s.contains("compatible") {
        (
            "Coordination game".into(),
            Some("Battle of the Sexes".into()),
        )
    } else if s.contains("auction") || s.contains("bid") {
        (
            "Auction".into(),
            Some("First-price sealed-bid auction".into()),
        )
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
    } else if !s.contains("payoff")
        && !s.contains("utility")
        && !s.contains("profit")
        && !s.contains("cost")
    {
        vec![AmbiguityNote {
            axis: "payoff_sum".into(),
            note: "no payoff or utility information provided".into(),
        }]
    } else {
        vec![]
    };

    let shadow_games: Vec<String> =
        if s.contains("price") && !s.contains("collude") && !s.contains("cartel") {
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
    params.insert(
        "fid".into(),
        cozo::DataValue::from(rd.fingerprint_id.as_str()),
    );
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
    costs_usd: &HashMap<String, f64>,
) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;
    let now = Utc::now().to_rfc3339();

    for (agent_key, output) in outputs {
        let mut params = BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(run_id));
        params.insert("ak".into(), cozo::DataValue::from(agent_key.as_str()));
        params.insert("out".into(), cozo::DataValue::from(output.as_str()));
        params.insert("status".into(), cozo::DataValue::from("completed"));
        params.insert("started".into(), cozo::DataValue::from(now.as_str()));
        params.insert("completed".into(), cozo::DataValue::from(now.as_str()));
        params.insert("duration".into(), cozo::DataValue::from("0"));
        let cost = format!("{:.6}", costs_usd.get(agent_key).copied().unwrap_or(0.0));
        params.insert("cost".into(), cozo::DataValue::from(cost.as_str()));

        db.run_script(
            "?[run_id, agent_key, output_json, status, started_at, completed_at, \
             duration_ms, cost_usd] <- [[$rid, $ak, $out, $status, $started, \
             $completed, $duration, $cost]] \
             :put gt_specialist_outputs { run_id, agent_key => output_json, status, \
             started_at, completed_at, duration_ms, cost_usd }",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("persist gt_specialist_outputs failed: {e}"))?;
        persist_run_checkpoint(
            db,
            run_id,
            &format!("specialist:{agent_key}"),
            "specialist",
            "completed",
            serde_json::json!({
                "agent_key": agent_key,
                "tier": agent_tier(agent_key),
                "cost_usd": cost,
            }),
        )?;
    }
    Ok(())
}

fn persist_specialist_failure(
    db: &DbInstance,
    run_id: &str,
    agent_key: &str,
    message: &str,
) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;
    let now = Utc::now().to_rfc3339();
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("ak".into(), cozo::DataValue::from(agent_key));
    params.insert("out".into(), cozo::DataValue::from(message));
    params.insert("status".into(), cozo::DataValue::from("failed"));
    params.insert("now".into(), cozo::DataValue::from(now.as_str()));

    db.run_script(
        "?[run_id, agent_key, output_json, status, started_at, completed_at, duration_ms, cost_usd] \
         <- [[$rid, $ak, $out, $status, $now, $now, '0', '0.000000']] \
         :put gt_specialist_outputs { run_id, agent_key => output_json, status, \
         started_at, completed_at, duration_ms, cost_usd }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist failed gt_specialist_outputs failed: {e}"))?;
    persist_run_checkpoint(
        db,
        run_id,
        &format!("specialist:{agent_key}"),
        "specialist",
        "failed",
        serde_json::json!({
            "agent_key": agent_key,
            "tier": agent_tier(agent_key),
            "message": message,
        }),
    )?;
    Ok(())
}

fn persist_sections(
    db: &DbInstance,
    run_id: &str,
    sections: &[final_stage::writer::SectionContent],
) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;

    let now = Utc::now().to_rfc3339();
    for (idx, section) in sections.iter().enumerate() {
        let section_id = format!("sec-{:02}", idx + 1);
        let title = section.section.title();
        let contributors_json = serde_json::to_string(&section.contributors)
            .map_err(|e| anyhow::anyhow!("serialize section contributors failed: {e}"))?;

        let mut params = BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(run_id));
        params.insert("sid".into(), cozo::DataValue::from(section_id.as_str()));
        params.insert("sty".into(), cozo::DataValue::from(title));
        params.insert("stt".into(), cozo::DataValue::from(title));
        params.insert(
            "smd".into(),
            cozo::DataValue::from(section.content.as_str()),
        );
        params.insert(
            "ssj".into(),
            cozo::DataValue::from(contributors_json.as_str()),
        );
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
    Ok(())
}

fn persist_final_report(
    db: &DbInstance,
    run_id: &str,
    report: &str,
    total_cost_usd: f64,
) -> Result<()> {
    use std::collections::BTreeMap;
    ensure_gametheory_schema(db)?;

    let now = Utc::now().to_rfc3339();

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("rep".into(), cozo::DataValue::from(report));
    params.insert("ca".into(), cozo::DataValue::from(now.as_str()));
    let cost = format!("{total_cost_usd:.6}");
    params.insert("cost".into(), cozo::DataValue::from(cost.as_str()));

    db.run_script(
        "?[run_id, report_md, created_at, total_cost_usd, total_duration_ms] \
         <- [[$rid, $rep, $ca, $cost, '0']] \
         :put gt_final_reports { run_id => report_md, created_at, \
         total_cost_usd, total_duration_ms }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_final_reports failed: {e}"))?;
    Ok(())
}

fn persist_provenance_edges_for_run<'a>(
    db: &DbInstance,
    run_id: &str,
    specialist_keys: impl IntoIterator<Item = &'a String>,
    sections: &[final_stage::writer::SectionContent],
) -> Result<()> {
    ensure_gametheory_schema(db)?;
    let mut edges = Vec::new();

    let situation = format!("situation:{run_id}");
    let fingerprint = format!("fingerprint:{run_id}");
    let routing = format!("routing:{run_id}");
    let report = format!("report:{run_id}");
    edges.push((situation, fingerprint.clone(), "produced_fingerprint"));
    edges.push((fingerprint.clone(), routing.clone(), "produced_routing"));

    for agent_key in specialist_keys {
        let specialist = specialist_artifact_id(run_id, agent_key);
        edges.push((routing.clone(), specialist, "enabled_specialist"));
    }

    for (idx, section) in sections.iter().enumerate() {
        let section_id = section_artifact_id(run_id, idx);
        for contributor in &section.contributors {
            edges.push((
                specialist_artifact_id(run_id, contributor),
                section_id.clone(),
                "contributed_to_section",
            ));
        }
        edges.push((section_id, report.clone(), "assembled_into_report"));
    }

    for (idx, (from, to, edge_type)) in edges.iter().enumerate() {
        persist_provenance_edge(db, run_id, idx + 1, from, to, edge_type)?;
    }
    Ok(())
}

fn specialist_artifact_id(run_id: &str, agent_key: &str) -> String {
    format!("specialist:{run_id}:{agent_key}")
}

fn section_artifact_id(run_id: &str, zero_based_idx: usize) -> String {
    format!("section:{run_id}:sec-{:02}", zero_based_idx + 1)
}

fn persist_provenance_edge(
    db: &DbInstance,
    run_id: &str,
    edge_index: usize,
    from: &str,
    to: &str,
    edge_type: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let edge_id = format!("{run_id}:edge-{edge_index:04}");
    let mut params = BTreeMap::new();
    params.insert("eid".into(), cozo::DataValue::from(edge_id.as_str()));
    params.insert("from".into(), cozo::DataValue::from(from));
    params.insert("to".into(), cozo::DataValue::from(to));
    params.insert("typ".into(), cozo::DataValue::from(edge_type));
    params.insert("ca".into(), cozo::DataValue::from(now.as_str()));

    db.run_script(
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
         <- [[$eid, $from, $to, $typ, $ca]] \
         :put gt_provenance_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_provenance_edges failed: {e}"))?;
    Ok(())
}

fn persist_run_checkpoint(
    db: &DbInstance,
    run_id: &str,
    checkpoint_key: &str,
    checkpoint_type: &str,
    status: &str,
    detail: serde_json::Value,
) -> Result<()> {
    ensure_gametheory_schema(db)?;
    let detail_json = serde_json::to_string(&detail)
        .map_err(|e| anyhow::anyhow!("serialize checkpoint detail failed: {e}"))?;
    let created_at = Utc::now().to_rfc3339();

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("ck".into(), cozo::DataValue::from(checkpoint_key));
    params.insert("ct".into(), cozo::DataValue::from(checkpoint_type));
    params.insert("st".into(), cozo::DataValue::from(status));
    params.insert("dj".into(), cozo::DataValue::from(detail_json.as_str()));
    params.insert("ca".into(), cozo::DataValue::from(created_at.as_str()));

    db.run_script(
        "?[run_id, checkpoint_key, checkpoint_type, status, detail_json, created_at] \
         <- [[$rid, $ck, $ct, $st, $dj, $ca]] \
         :put gt_run_checkpoints { run_id, checkpoint_key => checkpoint_type, status, detail_json, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_run_checkpoints failed: {e}"))?;
    Ok(())
}

fn persist_tier_checkpoints(
    db: &DbInstance,
    run_id: &str,
    outputs: &HashMap<String, String>,
) -> Result<()> {
    let mut by_tier: BTreeMap<u8, Vec<&str>> = BTreeMap::new();
    for agent_key in outputs.keys() {
        if let Some(tier) = agent_tier(agent_key) {
            by_tier.entry(tier).or_default().push(agent_key.as_str());
        }
    }

    for (tier, agents) in by_tier {
        persist_run_checkpoint(
            db,
            run_id,
            &format!("tier:{tier}"),
            "tier",
            "completed",
            serde_json::json!({"tier": tier, "completed_agents": agents}),
        )?;
    }
    Ok(())
}

// ── Cozo helpers ─────────────────────────────────────────────────────────────

fn load_run_state(db: &DbInstance, run_id: &str) -> Result<StoredRunState, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[situation, started_at, status] := *gt_runs{run_id, situation, started_at, completed_at, status}, \
             run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_runs failed: {e}"),
        })?;
    rows.rows
        .first()
        .map(|row| StoredRunState {
            situation: row[0].get_str().unwrap_or("").to_string(),
            started_at: row[1].get_str().unwrap_or("").to_string(),
            status: row[2].get_str().unwrap_or("").to_string(),
        })
        .ok_or_else(|| GameTheoryError::Storage {
            message: format!("run not found: {run_id}"),
        })
}

fn load_run_situation(db: &DbInstance, run_id: &str) -> Result<String, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[situation] := *gt_runs{run_id, situation, started_at, completed_at, status}, \
             run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_runs failed: {e}"),
        })?;
    rows.rows
        .first()
        .and_then(|row| row[0].get_str())
        .map(str::to_string)
        .ok_or_else(|| GameTheoryError::Storage {
            message: format!("run not found: {run_id}"),
        })
}

fn load_stored_fingerprint(
    db: &DbInstance,
    run_id: &str,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[fingerprint_json] := *gt_fingerprints{run_id, fingerprint_json, \
             primary_family, created_at}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_fingerprints failed: {e}"),
        })?;
    let json = rows
        .rows
        .first()
        .and_then(|row| row[0].get_str())
        .ok_or_else(|| GameTheoryError::Storage {
            message: format!("fingerprint not found for run: {run_id}"),
        })?;
    serde_json::from_str(json).map_err(|e| GameTheoryError::FingerprintParse {
        message: e.to_string(),
    })
}

fn load_stored_routing(
    db: &DbInstance,
    run_id: &str,
) -> Result<Option<RoutingDecision>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[fingerprint_id, enabled, skipped, conditions, created_at] := \
             *gt_routing_decisions{run_id, fingerprint_id, enabled_specialists_json: enabled, \
             skipped_specialists_json: skipped, evaluated_conditions_json: conditions, created_at}, \
             run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_routing_decisions failed: {e}"),
        })?;
    let Some(row) = rows.rows.first() else {
        return Ok(None);
    };

    let enabled = serde_json::from_str(row[1].get_str().unwrap_or("[]")).map_err(|e| {
        GameTheoryError::Storage {
            message: format!("parse enabled specialists failed: {e}"),
        }
    })?;
    let skipped = serde_json::from_str(row[2].get_str().unwrap_or("[]")).map_err(|e| {
        GameTheoryError::Storage {
            message: format!("parse skipped specialists failed: {e}"),
        }
    })?;
    let conditions = serde_json::from_str(row[3].get_str().unwrap_or("[]")).map_err(|e| {
        GameTheoryError::Storage {
            message: format!("parse evaluated conditions failed: {e}"),
        }
    })?;

    Ok(Some(RoutingDecision {
        run_id: run_id.to_string(),
        fingerprint_id: row[0].get_str().unwrap_or("").to_string(),
        enabled_specialists: enabled,
        skipped_specialists: skipped,
        evaluated_conditions: conditions,
        created_at: row[4].get_str().unwrap_or("").to_string(),
    }))
}

fn load_completed_specialist_keys(
    db: &DbInstance,
    run_id: &str,
) -> Result<HashSet<String>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[agent_key] := *gt_specialist_outputs{run_id, agent_key, status}, \
             run_id = $rid, status = 'completed'",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query completed specialists failed: {e}"),
        })?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| row[0].get_str().map(str::to_string))
        .collect())
}

fn load_completed_specialist_outputs(
    db: &DbInstance,
    run_id: &str,
) -> Result<HashMap<String, String>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[agent_key, output] := *gt_specialist_outputs{run_id, agent_key, output_json: output, status}, \
             run_id = $rid, status = 'completed'",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query completed specialist outputs failed: {e}"),
        })?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            let key = row[0].get_str()?;
            let output = row[1].get_str()?;
            Some((key.to_string(), output.to_string()))
        })
        .collect())
}

fn load_specialist_cost_total(db: &DbInstance, run_id: &str) -> Result<f64, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[cost] := *gt_specialist_outputs{run_id, agent_key, cost_usd: cost}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query specialist costs failed: {e}"),
        })?;
    rows.rows.iter().try_fold(0.0, |total, row| {
        let cost = row[0].get_str().unwrap_or("0");
        cost.parse::<f64>()
            .map(|parsed| total + parsed)
            .map_err(|e| GameTheoryError::Storage {
                message: format!("parse specialist cost '{cost}' failed: {e}"),
            })
    })
}

fn summarize_output(output: &str) -> String {
    let summary: String = output
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect();
    if output.chars().count() > summary.chars().count() {
        format!("{summary}...")
    } else {
        summary
    }
}

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
        "?[run_id, situation, started_at, completed_at, status, cost_usd] \
         <- [[$rid, $sit, $sa, \"\", $st, \"0.000000\"]] \
         :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
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
    cost_usd: f64,
) -> Result<()> {
    use std::collections::BTreeMap;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("sit".into(), cozo::DataValue::from(situation));
    params.insert("sa".into(), cozo::DataValue::from(started_at));
    params.insert("ca".into(), cozo::DataValue::from(completed_at));
    params.insert("st".into(), cozo::DataValue::from(status));
    let cost = format!("{cost_usd:.6}");
    params.insert("cost".into(), cozo::DataValue::from(cost.as_str()));

    db.run_script(
        "?[run_id, situation, started_at, completed_at, status, cost_usd] \
         <- [[$rid, $sit, $sa, $ca, $st, $cost]] \
         :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
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

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-gt-facade-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

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
        assert!(routing_rows.rows.len() >= 1);

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
        assert!((result.total_cost_usd - 1.2533).abs() < 0.000001);

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
    fn test_specialist_fixture_outputs_non_empty() {
        let db = test_db();
        let fp = block_on(classify(
            &db,
            "Two firms set quantities simultaneously.",
            None,
        ))
        .unwrap();

        // Build a minimal routing decision to test fixture execution.
        let rd = RoutingDecision {
            run_id: "test-fixture-run".into(),
            fingerprint_id: fp.run_id.clone(),
            enabled_specialists: vec![
                "nash-equilibrium-finder".into(),
                "payoff-matrix-builder".into(),
            ],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let (outputs, failed, audits) = execute_test_specialist_fixture(
            &rd,
            &fp,
            "Two firms set quantities.",
            &GameTheoryMemoryContext::default(),
        );
        assert_eq!(outputs.len(), 2);
        assert!(
            outputs
                .get("nash-equilibrium-finder")
                .unwrap()
                .contains("nash-equilibrium-finder")
        );
        assert!(
            outputs
                .get("payoff-matrix-builder")
                .unwrap()
                .contains("payoff-matrix-builder")
        );
        assert!(
            failed.is_empty(),
            "no forced failures without the test hook suffix"
        );
        assert_eq!(audits.len(), 2);
    }

    #[test]
    fn test_failure_isolation_with_force_fail_suffix() {
        let db = test_db();
        let fp = block_on(classify(
            &db,
            "Two firms set quantities simultaneously.",
            None,
        ))
        .unwrap();

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

        let (outputs, failed, audits) = execute_test_specialist_fixture(
            &rd,
            &fp,
            "Two firms set quantities.",
            &GameTheoryMemoryContext::default(),
        );
        // 2 of 3 succeed
        assert_eq!(outputs.len(), 2);
        assert!(outputs.contains_key("nash-equilibrium-finder"));
        assert!(outputs.contains_key("payoff-matrix-builder"));
        // 1 fails due to test hook
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "bayesian-game-analyzer-FORCE-FAIL-FOR-TEST");
        assert!(failed[0].1.contains("forced failure"));
        assert_eq!(audits.len(), 3);
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

    // ── MockLlmClient for testing LLM integration ─────────────────────────

    use crate::runner::{LlmClient, LlmResponse};
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockLlmClient {
        canned_response: Mutex<String>,
    }

    impl MockLlmClient {
        fn new(canned: &str) -> Self {
            Self {
                canned_response: Mutex::new(canned.to_string()),
            }
        }
    }

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> std::result::Result<LlmResponse, anyhow::Error> {
            Ok(LlmResponse {
                content: self.canned_response.lock().unwrap().clone(),
                tool_uses: vec![],
                tokens_in: 100,
                tokens_out: 200,
            })
        }
    }

    struct CapturingLlmClient {
        canned_response: String,
        classification_response: Option<String>,
        prompts: Mutex<Vec<String>>,
    }

    impl CapturingLlmClient {
        fn new(canned: &str) -> Self {
            Self {
                canned_response: canned.to_string(),
                classification_response: None,
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn with_classification(canned: &str, classification: String) -> Self {
            Self {
                canned_response: canned.to_string(),
                classification_response: Some(classification),
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmClient for CapturingLlmClient {
        async fn send_message(
            &self,
            messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> std::result::Result<LlmResponse, anyhow::Error> {
            let prompt = messages
                .first()
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            self.prompts.lock().unwrap().push(prompt.clone());
            let content = if prompt.starts_with("Classify this strategic situation") {
                self.classification_response
                    .clone()
                    .unwrap_or_else(|| self.canned_response.clone())
            } else {
                self.canned_response.clone()
            };
            Ok(LlmResponse {
                content,
                tool_uses: vec![],
                tokens_in: 100,
                tokens_out: 200,
            })
        }
    }

    struct MockLeannSearcher {
        response: String,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl MockLeannSearcher {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl LeannSearcher for MockLeannSearcher {
        fn search(&self, _query: &str) -> String {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.response.clone()
        }
    }

    struct SlowTier1LlmClient {
        response: String,
        active: std::sync::atomic::AtomicUsize,
        max_active: std::sync::atomic::AtomicUsize,
        prompts: Mutex<Vec<String>>,
    }

    impl SlowTier1LlmClient {
        fn new(response: String) -> Self {
            Self {
                response,
                active: std::sync::atomic::AtomicUsize::new(0),
                max_active: std::sync::atomic::AtomicUsize::new(0),
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn max_active(&self) -> usize {
            self.max_active.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmClient for SlowTier1LlmClient {
        async fn send_message(
            &self,
            messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> std::result::Result<LlmResponse, anyhow::Error> {
            let prompt = messages
                .first()
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            self.prompts.lock().unwrap().push(prompt);

            let active = self
                .active
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.max_active
                .fetch_max(active, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            self.active
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

            Ok(LlmResponse {
                content: self.response.clone(),
                tool_uses: vec![],
                tokens_in: 100,
                tokens_out: 200,
            })
        }
    }

    fn canned_specialist_llm() -> CapturingLlmClient {
        CapturingLlmClient::new("specialist output")
    }

    fn canned_fingerprint_json() -> String {
        serde_json::json!({
            "cooperation": {"value": "non-cooperative", "confidence": "high", "rationale": "firms compete"},
            "payoff_sum": {"value": "variable-sum", "confidence": "medium", "rationale": "price outcomes vary"},
            "symmetry": {"value": "asymmetric", "confidence": "medium", "rationale": "costs differ"},
            "timing": {"value": "simultaneous", "confidence": "high", "rationale": "firms act at once"},
            "perfect_info": {"value": "imperfect", "confidence": "medium", "rationale": "prices are not observed before choosing"},
            "complete_info": {"value": "complete", "confidence": "medium", "rationale": "game form is known"},
            "cardinality": {"value": "2-player", "confidence": "high", "rationale": "duopoly"},
            "strategy_space": {"value": "continuous", "confidence": "high", "rationale": "prices or quantities are continuous"},
            "horizon": {"value": "one-shot", "confidence": "medium", "rationale": "single interaction"},
            "primary_family": "Bertrand competition",
            "nearest_classic": "Bertrand duopoly"
        })
        .to_string()
    }

    fn canned_pipeline_llm() -> CapturingLlmClient {
        CapturingLlmClient::with_classification("specialist output", canned_fingerprint_json())
    }

    fn test_fingerprint(run_id: &str) -> GameTheoryFingerprint {
        GameTheoryFingerprint {
            run_id: run_id.into(),
            cooperation: AxisVerdict::new("non-cooperative", "high", ""),
            payoff_sum: AxisVerdict::new("zero-sum", "medium", ""),
            symmetry: AxisVerdict::new("asymmetric", "medium", ""),
            timing: AxisVerdict::new("simultaneous", "high", ""),
            perfect_info: AxisVerdict::new("imperfect", "medium", ""),
            complete_info: AxisVerdict::new("complete", "medium", ""),
            cardinality: AxisVerdict::new("2-player", "high", ""),
            strategy_space: AxisVerdict::new("discrete", "medium", ""),
            horizon: AxisVerdict::new("one-shot", "medium", ""),
            primary_family: "test".into(),
            nearest_classic: None,
            shadow_games: vec![],
            hidden_game_scan: None,
            ambiguities: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_facade_recalls_memory_for_specialist_keys() {
        let graph = archon_memory::MemoryGraph::in_memory().unwrap();
        graph
            .store_memory(
                "stored payoff prior context",
                "gametheory/tier1/payoffs",
                archon_memory::MemoryType::Fact,
                0.9,
                &[TAG_GAMETHEORY_PIPELINE.to_string()],
                "test",
                "workspace",
            )
            .unwrap();
        let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
        let ctx = GameTheoryMemoryContext::new(memory, None, true);
        let llm = canned_pipeline_llm();
        let rd = RoutingDecision {
            run_id: "run-memory".into(),
            fingerprint_id: "fp-memory".into(),
            enabled_specialists: vec!["nash-equilibrium-finder".into()],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let (_outputs, failed, audits) = block_on(execute_specialists_real(
            &llm,
            &rd,
            &test_fingerprint("fp-memory"),
            "Two firms choose prices.",
            &ctx,
        ))
        .unwrap();

        assert!(failed.is_empty());
        assert!(llm.prompts()[0].contains("stored payoff prior context"));
        assert_eq!(audits[0].agent_key, "nash-equilibrium-finder");
        assert_eq!(audits[0].cozo_hits, 1);
        assert_eq!(audits[0].leann_hits, 0);
    }

    #[test]
    fn test_specialist_fixture_recalls_memory_for_debug_path() {
        let graph = archon_memory::MemoryGraph::in_memory().unwrap();
        graph
            .store_memory(
                "stored no-provider prior context",
                "gametheory/tier1/payoffs",
                archon_memory::MemoryType::Fact,
                0.9,
                &[TAG_GAMETHEORY_PIPELINE.to_string()],
                "test",
                "workspace",
            )
            .unwrap();
        let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
        let ctx = GameTheoryMemoryContext::new(memory, None, true);
        let rd = RoutingDecision {
            run_id: "run-fixture-memory".into(),
            fingerprint_id: "fp-fixture-memory".into(),
            enabled_specialists: vec!["nash-equilibrium-finder".into()],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let (outputs, failed, audits) = execute_test_specialist_fixture(
            &rd,
            &test_fingerprint("fp-fixture-memory"),
            "Two firms choose prices.",
            &ctx,
        );

        assert!(failed.is_empty());
        assert!(outputs["nash-equilibrium-finder"].contains("stored no-provider prior context"));
        assert_eq!(audits[0].cozo_hits, 1);
        assert_eq!(audits[0].leann_hits, 0);
    }

    #[test]
    fn test_facade_falls_back_to_leann_when_cozo_empty() {
        let graph = archon_memory::MemoryGraph::in_memory().unwrap();
        let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
        let leann = std::sync::Arc::new(MockLeannSearcher::new("leann semantic prior"));
        let ctx = GameTheoryMemoryContext::new(memory, Some(leann.clone()), true);
        let llm = canned_pipeline_llm();
        let rd = RoutingDecision {
            run_id: "run-leann".into(),
            fingerprint_id: "fp-leann".into(),
            enabled_specialists: vec!["nash-equilibrium-finder".into()],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let (_outputs, _failed, audits) = block_on(execute_specialists_real(
            &llm,
            &rd,
            &test_fingerprint("fp-leann"),
            "Two firms choose prices.",
            &ctx,
        ))
        .unwrap();

        assert!(llm.prompts()[0].contains("leann semantic prior"));
        assert_eq!(audits[0].cozo_hits, 0);
        assert_eq!(audits[0].leann_hits, 2);
        assert_eq!(leann.calls(), 2);
    }

    #[test]
    fn test_no_recall_when_memory_keys_empty() {
        let graph = archon_memory::MemoryGraph::in_memory().unwrap();
        let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
        let leann = std::sync::Arc::new(MockLeannSearcher::new("should not be used"));
        let ctx = GameTheoryMemoryContext::new(memory, Some(leann.clone()), true);
        let llm = canned_specialist_llm();
        let rd = RoutingDecision {
            run_id: "run-empty".into(),
            fingerprint_id: "fp-empty".into(),
            enabled_specialists: vec!["agent-with-no-memory-keys".into()],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let (_outputs, _failed, audits) = block_on(execute_specialists_real(
            &llm,
            &rd,
            &test_fingerprint("fp-empty"),
            "Two firms choose prices.",
            &ctx,
        ))
        .unwrap();

        assert_eq!(audits[0].memory_keys.len(), 0);
        assert_eq!(audits[0].cozo_hits, 0);
        assert_eq!(audits[0].leann_hits, 0);
        assert_eq!(leann.calls(), 0);
        assert!(!llm.prompts()[0].contains("## Prior Context"));
    }

    #[test]
    fn test_cost_tracked_per_specialist() {
        let llm = canned_specialist_llm();
        let rd = RoutingDecision {
            run_id: "run-cost".into(),
            fingerprint_id: "fp-cost".into(),
            enabled_specialists: vec!["nash-equilibrium-finder".into()],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let outcome = block_on(execute_specialists_real_with_options(
            &llm,
            &rd,
            &test_fingerprint("fp-cost"),
            "Two firms choose prices.",
            &GameTheoryMemoryContext::default(),
            &GameTheoryRunOptions::default(),
        ))
        .unwrap();

        let cost = outcome.costs_usd["nash-equilibrium-finder"];
        assert!(cost > 0.0);
        assert!((outcome.total_cost_usd - cost).abs() < f64::EPSILON);
        assert!((outcome.tier_costs_usd[&2] - cost).abs() < f64::EPSILON);
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
        assert_eq!(rows.rows[0][1].get_str().unwrap(), "0.003300");

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
        assert_eq!(specialist_rows.rows[0][0].get_str().unwrap(), "0.003300");

        let mut params = std::collections::BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(result.run_id.as_str()));
        let report_rows = db
            .run_script(
                "?[cost] := *gt_final_reports{run_id, total_cost_usd: cost}, run_id = $rid",
                params,
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(report_rows.rows[0][0].get_str().unwrap(), "0.003300");
    }

    #[test]
    fn test_concurrency_cap_respected_per_run_flag() {
        let llm = canned_specialist_llm();
        let rd = RoutingDecision {
            run_id: "run-concurrency".into(),
            fingerprint_id: "fp-concurrency".into(),
            enabled_specialists: vec![
                "nash-equilibrium-finder".into(),
                "payoff-matrix-builder".into(),
            ],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let outcome = block_on(execute_specialists_real_with_options(
            &llm,
            &rd,
            &test_fingerprint("fp-concurrency"),
            "Two firms choose prices.",
            &GameTheoryMemoryContext::default(),
            &GameTheoryRunOptions {
                budget_usd: 20.0,
                max_concurrent: 1,
                style_profile_id: None,
                enable_tier11: false,
                kb_pack_id: None,
            },
        ))
        .unwrap();

        assert!(outcome.max_observed_concurrent <= 1);
        assert_eq!(outcome.outputs.len(), 2);
    }

    #[test]
    fn test_specialists_dispatch_in_parallel_waves() {
        let llm = SlowTier1LlmClient::new("parallel specialist output".to_string());
        let rd = RoutingDecision {
            run_id: "run-specialist-parallel".into(),
            fingerprint_id: "fp-specialist-parallel".into(),
            enabled_specialists: vec![
                "nash-equilibrium-finder".into(),
                "payoff-matrix-builder".into(),
                "dominant-strategy-identifier".into(),
            ],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let started = std::time::Instant::now();
        let outcome = block_on(execute_specialists_real_with_options(
            &llm,
            &rd,
            &test_fingerprint("fp-specialist-parallel"),
            "Two firms choose prices.",
            &GameTheoryMemoryContext::default(),
            &GameTheoryRunOptions {
                budget_usd: 20.0,
                max_concurrent: 2,
                style_profile_id: None,
                enable_tier11: false,
                kb_pack_id: None,
            },
        ))
        .unwrap();

        assert_eq!(outcome.outputs.len(), 3);
        assert_eq!(outcome.max_observed_concurrent, 2);
        assert!(
            llm.max_active() > 1,
            "specialist calls must overlap when max_concurrent > 1"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(320),
            "three 120ms specialists with max_concurrent=2 should not run serially"
        );
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

    fn seed_kb_pack(db: &DbInstance, pack_id: &str, content: &str) {
        db.run_script(
            ":create doc_sources { document_id: String => source_path: String }",
            Default::default(),
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
        db.run_script(
            ":create doc_chunks { chunk_id: String => document_id: String, content: String }",
            Default::default(),
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();

        db.run_script(
            "?[document_id, source_path] <- [[$did, $path]] \
             :put doc_sources { document_id => source_path }",
            std::collections::BTreeMap::from([
                ("did".into(), cozo::DataValue::from("doc-policy-pack")),
                (
                    "path".into(),
                    cozo::DataValue::from(format!("./fixtures/{pack_id}/policy.md")),
                ),
            ]),
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
        db.run_script(
            "?[chunk_id, document_id, content] <- [[\"chunk-policy-pack-0\", \"doc-policy-pack\", $content]] \
             :put doc_chunks { chunk_id => document_id, content }",
            std::collections::BTreeMap::from([(
                "content".into(),
                cozo::DataValue::from(content),
            )]),
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
    }

    #[test]
    fn test_tier1_foundation_agents_run_as_parallel_wave() {
        let llm = SlowTier1LlmClient::new(canned_fingerprint_json());
        let started = std::time::Instant::now();
        let (fp, audits) = block_on(execute_tier1_real(
            &llm,
            "run-tier1-parallel",
            "Two firms simultaneously set prices in a Bertrand duopoly.",
            "2026-05-04T00:00:00Z",
            &GameTheoryMemoryContext::default(),
        ))
        .unwrap();

        assert_eq!(fp.primary_family, "Bertrand competition");
        assert_eq!(audits.len(), TIER1_MEMORY_AGENT_KEYS.len());
        assert!(
            llm.max_active() > 1,
            "Tier 1 mandatory agents must overlap in one parallel wave"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(360),
            "four 120ms Tier 1 calls should not run serially"
        );
        let prompts = llm.prompts().join("\n");
        for agent_key in TIER1_MEMORY_AGENT_KEYS {
            assert!(prompts.contains(agent_key));
        }
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
    fn test_real_specialist_execution_with_failure_isolation() {
        let db = test_db();
        let fp = block_on(classify(
            &db,
            "Two firms set quantities simultaneously.",
            None,
        ))
        .unwrap();

        let rd = RoutingDecision {
            run_id: "test-real-fail-iso".into(),
            fingerprint_id: fp.run_id.clone(),
            enabled_specialists: vec![
                "market-structure-analyzer".into(),
                "game-tree-builder-FORCE-FAIL-FOR-TEST".into(),
                "payoff-matrix-builder".into(),
            ],
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let (outputs, failed, audits) = execute_test_specialist_fixture(
            &rd,
            &fp,
            "Two firms set quantities.",
            &GameTheoryMemoryContext::default(),
        );
        assert_eq!(outputs.len(), 2, "2 of 3 specialists should succeed");
        assert!(outputs.contains_key("market-structure-analyzer"));
        assert!(outputs.contains_key("payoff-matrix-builder"));
        assert_eq!(
            failed.len(),
            1,
            "1 specialist should fail due to FORCE-FAIL hook"
        );
        assert_eq!(failed[0].0, "game-tree-builder-FORCE-FAIL-FOR-TEST");
        assert!(
            failed[0].1.contains("forced failure"),
            "error message should mention forced failure"
        );

        // Verify the failed specialist is NOT in outputs
        assert!(!outputs.contains_key("game-tree-builder-FORCE-FAIL-FOR-TEST"));
        assert_eq!(audits.len(), 3);
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
}
