//! `/pipeline` slash command handler.
//! Extracted from main.rs to reduce main.rs from 1532 to < 500 lines.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_memory::{MemoryTrait, graph::MemoryGraph};

use crate::cli_args::PipelineAction;
use crate::command::pipeline_support::{
    build_pipeline_adapter, init_leann, init_research_leann, print_pipeline_result,
};
use crate::command::provider_gate::ensure_active_provider_supports;

/// Handle `/archon pipeline` subcommands.
pub async fn handle_pipeline_command(
    action: &PipelineAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> std::result::Result<(), anyhow::Error> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match action {
        PipelineAction::Code { task, dry_run } => {
            handle_code(task, *dry_run, &cwd, config, env_vars).await?;
        }
        PipelineAction::Research { topic, dry_run } => {
            handle_research(topic, *dry_run, &cwd, config, env_vars).await?;
        }
        PipelineAction::Status { session_id } => {
            crate::command::pipeline_bundle::handle_status(&cwd, session_id).await?;
        }
        PipelineAction::Resume { session_id } => {
            crate::command::pipeline_bundle::handle_resume(&cwd, session_id, config, env_vars)
                .await?;
        }
        PipelineAction::List => {
            crate::command::pipeline_bundle::handle_list(&cwd).await?;
        }
        PipelineAction::Abort { session_id } => {
            crate::command::pipeline_bundle::handle_abort(&cwd, session_id).await?;
        }
        PipelineAction::Verify {
            session_id,
            write_report,
        } => {
            crate::command::pipeline_bundle::handle_verify(&cwd, session_id, *write_report).await?;
        }
        PipelineAction::Inspect { session_id } => {
            crate::command::pipeline_bundle::handle_inspect(&cwd, session_id).await?;
        }
        PipelineAction::ExportTraces {
            session_id,
            format,
            out,
            include_unverified,
        } => {
            crate::command::pipeline_bundle::handle_export_traces(
                &cwd,
                session_id,
                format,
                out.as_ref(),
                *include_unverified,
            )
            .await?;
        }
        PipelineAction::Run {
            file,
            format,
            detach,
        } => {
            crate::command::pipeline_declarative::handle_run(
                &cwd,
                file,
                format.as_deref(),
                *detach,
            )
            .await?;
        }
        PipelineAction::Cancel { id } => {
            crate::command::pipeline_declarative::handle_cancel(&cwd, id).await?;
        }
    }
    Ok(())
}

async fn handle_code(
    task: &str,
    dry_run: bool,
    cwd: &std::path::Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    if dry_run {
        println!("=== Coding Pipeline Dry Run ===");
        println!("Task: {task}");
        println!("\nAgent Sequence (50 agents):");
        println!("  Phase 1: task-analyzer, requirement-extractor, requirement-prioritizer");
        println!(
            "  Phase 2: pattern-explorer, technology-scout, feasibility-analyzer, codebase-analyzer"
        );
        println!("  Phase 3: system-designer, component-designer, interface-designer, ...");
        println!("  Phase 4: code-generator, unit-implementer, api-implementer, ...");
        println!("  Phase 5: test-generator, integration-tester, security-tester, ...");
        println!("  Phase 6: final-refactorer, sign-off-approver");
        println!("\nEstimated cost: ~$2.50-5.00 (varies by task complexity)");
        return Ok(());
    }

    ensure_active_provider_supports(
        config,
        archon_llm::providers::ProviderCapability::PipelineCoding,
        "archon pipeline code",
    )?;
    let world_guardrail = crate::command::world_model::begin_guarded_action(
        config,
        archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
        "pipeline-code",
        "pipeline_code_start",
        &format!("coding pipeline: {task}"),
    );
    let world_record = world_guardrail
        .as_ref()
        .map(|record| record.advisory.clone());
    let world_record = world_record.unwrap_or_else(|| {
        crate::command::world_model::record_runtime_advisory(
            config,
            archon_world_model::integration::WorldAdvisorSurface::Pipeline,
            "pipeline-code",
            "pipeline_code_start",
            task,
        )
    });
    tracing::debug!(
        continue_foreground_flow = world_record.continue_foreground_flow,
        "world_model.pipeline_advisory"
    );
    if let Some(record) = &world_guardrail
        && !record.decision.allowed_to_finalize
        && !record.decision.required_actions.is_empty()
    {
        println!(
            "World model guardrail: {:?} risk; pipeline completion requires {:?}.",
            record.decision.risk_tier, record.decision.required_actions
        );
    }
    let _ = crate::command::world_model::record_runtime_counterfactual_advice(
        config,
        archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
        task,
        &[
            ("pipeline-code", "run the full coding pipeline now"),
            ("verify-first", "run verification before coding"),
            ("resume-existing", "resume a previous coding pipeline"),
            ("surface-memory", "surface relevant memories before coding"),
            (
                "provider-fallback",
                "switch provider before pipeline execution",
            ),
        ],
    );
    let adapter = build_pipeline_adapter(config, env_vars, "pipeline_code").await?;
    let learning = archon_pipeline::learning::integration::LearningIntegration::new(
        None,
        None,
        Default::default(),
        None,
    );
    let facade = archon_pipeline::coding::facade::CodingFacade::with_learning(learning)
        .with_models(config.models.anthropic.clone())
        .with_context(config.context.clone());
    let leann = init_leann(cwd).await;
    println!("Starting coding pipeline...");
    println!("Task: {task}");
    let result = archon_pipeline::runner::run_pipeline_audited(
        &facade,
        &adapter,
        task,
        cwd,
        leann.as_ref(),
        None,
        None,
    )
    .await?;
    print_pipeline_result(&result, cwd).await;
    if let Some(record) = &world_guardrail {
        let step_report =
            crate::command::world_model::record_guardrail_pipeline_steps(config, record, &result);
        if step_report.steps_recorded > 0 {
            println!(
                "World model guardrail: recorded {} pipeline steps and {} verification signals.",
                step_report.steps_recorded, step_report.parent_verifications_recorded
            );
        }
        if let Some(outcome) = crate::command::world_model::record_guardrail_completion_outcome(
            config,
            record,
            true,
            &result.final_output,
            Some(&result.session_id),
        ) && matches!(
            outcome.final_status,
            archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
                | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
        ) {
            println!(
                "World model guardrail: pipeline output is not marked verified yet; required actions: {:?}",
                record.decision.required_actions
            );
        }
    } else {
        crate::command::world_model::record_runtime_outcome(
            config,
            &world_record,
            &result.final_output,
            Some(&result.session_id),
        );
    }
    crate::command::world_model::schedule_dynamic_trainer_tick(config.clone());
    Ok(())
}

async fn handle_research(
    topic: &str,
    dry_run: bool,
    cwd: &std::path::Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    if dry_run {
        println!("=== Research Pipeline Dry Run ===");
        println!("Topic: {topic}");
        println!("\nAgent Sequence (46 agents):");
        println!("  Phase 1-3: foundation, analysis, methodology");
        println!("  Phase 4-7: writing, verification");
        println!("  Final assembly: final-stage-orchestrator");
        println!("\nEstimated cost: ~$3.00-8.00 (varies by topic complexity)");
        return Ok(());
    }

    ensure_active_provider_supports(
        config,
        archon_llm::providers::ProviderCapability::PipelineResearch,
        "archon pipeline research",
    )?;
    let world_guardrail = crate::command::world_model::begin_guarded_action(
        config,
        archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
        "pipeline-research",
        "pipeline_research_start",
        &format!("research pipeline: {topic}"),
    );
    let world_record = world_guardrail
        .as_ref()
        .map(|record| record.advisory.clone());
    let world_record = world_record.unwrap_or_else(|| {
        crate::command::world_model::record_runtime_advisory(
            config,
            archon_world_model::integration::WorldAdvisorSurface::Pipeline,
            "pipeline-research",
            "pipeline_research_start",
            topic,
        )
    });
    tracing::debug!(
        continue_foreground_flow = world_record.continue_foreground_flow,
        "world_model.pipeline_advisory"
    );
    if let Some(record) = &world_guardrail
        && !record.decision.allowed_to_finalize
        && !record.decision.required_actions.is_empty()
    {
        println!(
            "World model guardrail: {:?} risk; pipeline completion requires {:?}.",
            record.decision.risk_tier, record.decision.required_actions
        );
    }
    let _ = crate::command::world_model::record_runtime_counterfactual_advice(
        config,
        archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
        topic,
        &[
            ("pipeline-research", "run the full research pipeline now"),
            ("verify-first", "run source verification before research"),
            ("resume-existing", "resume a previous research pipeline"),
            (
                "surface-memory",
                "surface relevant memories before research",
            ),
            (
                "provider-fallback",
                "switch provider before pipeline execution",
            ),
        ],
    );
    let adapter = build_pipeline_adapter(config, env_vars, "pipeline_research").await?;
    let phd_learning = archon_pipeline::learning::integration::PhDLearningIntegration::new();
    let memory: Arc<dyn MemoryTrait> =
        Arc::new(MemoryGraph::in_memory().expect("in-memory graph for research pipeline"));
    let leann = init_research_leann(cwd).await;
    let facade = archon_pipeline::research::facade::ResearchFacade::with_learning(
        memory,
        leann,
        cwd.display().to_string(),
        None,
        phd_learning,
    )
    .with_models(config.models.anthropic.clone())
    .with_context(config.context.clone());
    println!("Starting research pipeline...");
    println!("Topic: {topic}");
    let result = archon_pipeline::runner::run_pipeline_audited(
        &facade, &adapter, topic, cwd, None, None, None,
    )
    .await?;
    print_pipeline_result(&result, cwd).await;
    if let Some(record) = &world_guardrail {
        let step_report =
            crate::command::world_model::record_guardrail_pipeline_steps(config, record, &result);
        if step_report.steps_recorded > 0 {
            println!(
                "World model guardrail: recorded {} pipeline steps and {} verification signals.",
                step_report.steps_recorded, step_report.parent_verifications_recorded
            );
        }
        if let Some(outcome) = crate::command::world_model::record_guardrail_completion_outcome(
            config,
            record,
            true,
            &result.final_output,
            Some(&result.session_id),
        ) && matches!(
            outcome.final_status,
            archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
                | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
        ) {
            println!(
                "World model guardrail: pipeline output is not marked verified yet; required actions: {:?}",
                record.decision.required_actions
            );
        }
    } else {
        crate::command::world_model::record_runtime_outcome(
            config,
            &world_record,
            &result.final_output,
            Some(&result.session_id),
        );
    }
    crate::command::world_model::schedule_dynamic_trainer_tick(config.clone());
    Ok(())
}
