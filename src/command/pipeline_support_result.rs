//! Reporting a finished pipeline run: console summary, research artifact paths,
//! and the completion-integrity check written back to the bundle store.
//!
//! Split out of `pipeline_support.rs`, which crossed the 500-line ceiling. The
//! split follows the seam that was already there — everything here runs *after*
//! a pipeline completes and only reads its `PipelineResult`, whereas the rest of
//! `pipeline_support` builds the adapters and learning stack a run needs before
//! it starts.

use std::path::Path;

use anyhow::Result;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::audit::types::PipelineEvent;
use archon_pipeline::runner::PipelineType;
use chrono::Utc;

pub(crate) async fn print_pipeline_result(
    result: &archon_pipeline::runner::PipelineResult,
    cwd: &Path,
) {
    println!("\n=== Pipeline Complete ===");
    println!("Session: {}", result.session_id);
    println!("Agents run: {}", result.agent_results.len());
    println!("Total cost: ${:.4}", result.total_cost_usd);
    println!("Duration: {:.1}s", result.duration.as_secs_f64());
    if let Some((markdown_path, pdf_path)) = final_research_artifact_paths(result, cwd) {
        println!("Final paper Markdown: {}", markdown_path.display());
        println!("Final paper PDF: {}", pdf_path.display());
    }
    match completion_summary(result, cwd).await {
        Ok(Some(summary)) => println!("Completion integrity: {}", summary.text),
        Ok(None) => {}
        Err(error) => {
            println!("Completion integrity: unavailable ({error})");
        }
    }
}

pub(crate) fn final_research_artifact_paths(
    result: &archon_pipeline::runner::PipelineResult,
    cwd: &Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    if result.pipeline_type != PipelineType::Research {
        return None;
    }
    let bundle_dir = PipelineBundleStore::new(cwd).bundle_dir(&result.session_id);
    let (markdown_path, pdf_path) =
        archon_pipeline::research::final_artifact::artifact_paths(&bundle_dir);
    if markdown_path.exists() || pdf_path.exists() {
        Some((markdown_path, pdf_path))
    } else {
        None
    }
}

struct CompletionSummary {
    text: String,
}

async fn completion_summary(
    result: &archon_pipeline::runner::PipelineResult,
    cwd: &Path,
) -> Result<Option<CompletionSummary>> {
    if result.final_output.trim().is_empty() {
        return Ok(None);
    }
    if result.pipeline_type == PipelineType::Research {
        let store = PipelineBundleStore::new(cwd);
        if let Ok(mut state) = store.load_state(&result.session_id) {
            state.completion_integrity_summary = None;
            state.completion_report_id = None;
            state.updated_at = Utc::now();
            store.save_state(&state)?;
        }
        return Ok(None);
    }
    let db = crate::command::store_paths::open_evidence_db(
        "completion",
        &["ARCHON_COMPLETION_DB_PATH"],
    )?;
    let task_type = match result.pipeline_type {
        archon_pipeline::runner::PipelineType::Coding => "coding",
        archon_pipeline::runner::PipelineType::Research => "research",
        archon_pipeline::runner::PipelineType::Learning => "learning",
        archon_pipeline::runner::PipelineType::Kb => "kb",
        archon_pipeline::runner::PipelineType::GameTheory => "gametheory",
        archon_pipeline::runner::PipelineType::Workflow => "workflow",
    };
    let (agent_key, model) = result
        .agent_results
        .last()
        .map(|(agent, _)| (Some(agent.key.clone()), Some(agent.model.clone())))
        .unwrap_or((None, None));
    let report = archon_completion::check_completion_with_context(
        &db,
        &result.session_id,
        &result.final_output,
        task_type,
        archon_completion::CompletionContext {
            workspace_id: std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".into()),
            agent_key,
            model,
        },
    )
    .await?;
    let verified = report.claims.iter().filter(|claim| claim.verified).count();
    let summary = format!(
        "{:?}; {verified}/{} completion-sensitive claims verified",
        report.final_state,
        report.claims.len()
    );
    let store = PipelineBundleStore::new(cwd);
    if let Ok(mut state) = store.load_state(&result.session_id) {
        state.completion_integrity_summary = Some(summary.clone());
        state.completion_report_id = Some(report.report_id.clone());
        state.updated_at = Utc::now();
        store.save_state(&state)?;
        store.append_event(
            &result.session_id,
            PipelineEvent::CompletionChecked {
                final_state: format!("{:?}", report.final_state),
                claim_count: report.claims.len(),
                verified_claim_count: verified,
                report_id: report.report_id,
            },
        )?;
    }
    Ok(Some(CompletionSummary { text: summary }))
}
