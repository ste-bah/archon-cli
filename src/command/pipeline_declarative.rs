use std::path::Path;

use anyhow::Result;

pub(crate) async fn handle_run(
    cwd: &Path,
    file: &Path,
    format: Option<&str>,
    detach: bool,
) -> Result<()> {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to read {}: {e}", file.display());
            std::process::exit(3);
        }
    };
    let fmt = detect_format(file, format, &src)?;
    let store_path = cwd.join(".archon").join("pipelines");
    let _ = std::fs::create_dir_all(&store_path);
    let store = std::sync::Arc::new(archon_pipeline::PipelineStateStore::new(&store_path));
    let registry = std::sync::Arc::new(archon_core::agents::AgentRegistry::load(cwd));
    let task_service: std::sync::Arc<dyn archon_core::tasks::TaskService> =
        std::sync::Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let engine = archon_pipeline::DefaultPipelineEngine::new(store, task_service);
    let spec = match engine.parse(&src, fmt) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Parse error: {e}");
            std::process::exit(3);
        }
    };
    if let Err(e) = engine.validate(&spec) {
        eprintln!("Validation error: {e}");
        std::process::exit(3);
    }
    use archon_pipeline::PipelineEngine;
    match engine.run(spec).await {
        Ok(id) => {
            println!("{id}");
            if !detach {
                poll_pipeline_status(engine, id).await;
            }
        }
        Err(e) => {
            eprintln!("Pipeline failed: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_cancel(cwd: &Path, id: &str) -> Result<()> {
    let pipeline_id: archon_pipeline::PipelineId = match id.parse() {
        Ok(pid) => pid,
        Err(e) => {
            eprintln!("Invalid pipeline ID '{id}': {e}");
            std::process::exit(1);
        }
    };
    let store_path = cwd.join(".archon").join("pipelines");
    let store = std::sync::Arc::new(archon_pipeline::PipelineStateStore::new(&store_path));
    let registry = std::sync::Arc::new(archon_core::agents::AgentRegistry::load(cwd));
    let task_service: std::sync::Arc<dyn archon_core::tasks::TaskService> =
        std::sync::Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let engine = archon_pipeline::DefaultPipelineEngine::new(store, task_service);
    use archon_pipeline::PipelineEngine;
    match engine.cancel(pipeline_id).await {
        Ok(()) => println!("Pipeline {id} cancelled."),
        Err(e) => {
            eprintln!("Failed to cancel pipeline {id}: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn detect_format(
    file: &Path,
    format: Option<&str>,
    src: &str,
) -> Result<archon_pipeline::PipelineFormat> {
    match format {
        Some("yaml" | "yml") => Ok(archon_pipeline::PipelineFormat::Yaml),
        Some("json") => Ok(archon_pipeline::PipelineFormat::Json),
        Some(other) => {
            eprintln!("Unknown format: {other} (expected yaml or json)");
            std::process::exit(3);
        }
        None => match file.extension().and_then(|e| e.to_str()) {
            Some("json") => Ok(archon_pipeline::PipelineFormat::Json),
            Some("yaml" | "yml") => Ok(archon_pipeline::PipelineFormat::Yaml),
            _ => {
                if src.trim_start().starts_with('{') || src.trim_start().starts_with('[') {
                    Ok(archon_pipeline::PipelineFormat::Json)
                } else {
                    Ok(archon_pipeline::PipelineFormat::Yaml)
                }
            }
        },
    }
}

async fn poll_pipeline_status<E: archon_pipeline::PipelineEngine>(
    engine: E,
    id: archon_pipeline::PipelineId,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        match engine.status(id).await {
            Ok(run) => {
                let finished_count = run
                    .steps
                    .values()
                    .filter(|s| s.state == archon_pipeline::StepRunState::Finished)
                    .count();
                let total = run.steps.len();
                eprint!("\r[{}/{}] {:?}  ", finished_count, total, run.state);
                match run.state {
                    archon_pipeline::PipelineState::Finished => {
                        eprintln!();
                        break;
                    }
                    archon_pipeline::PipelineState::Failed
                    | archon_pipeline::PipelineState::RolledBack => {
                        eprintln!();
                        std::process::exit(1);
                    }
                    archon_pipeline::PipelineState::Cancelled => {
                        eprintln!();
                        std::process::exit(2);
                    }
                    _ => {}
                }
            }
            Err(e) => {
                eprintln!("\nFailed to get status: {e}");
                std::process::exit(1);
            }
        }
    }
}
