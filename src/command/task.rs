use std::sync::Arc;
use std::path::PathBuf;

pub(crate) async fn handle_task_status(
    task_id: &str,
    watch: bool,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    match api.status(task_id, watch).await {
        Ok(output) => println!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_task_result(
    task_id: &str,
    stream: bool,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    match api.result(task_id, stream).await {
        Ok(output) => println!("{output}"),
        Err(archon_core::tasks::TaskError::Pending) => {
            eprintln!("TASK_PENDING: task has not completed yet");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_task_cancel(task_id: &str, working_dir: &PathBuf) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    match api.cancel(task_id).await {
        Ok(output) => println!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_task_list(
    state: Option<String>,
    agent: Option<String>,
    since: Option<String>,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    match api.list(state, agent, since).await {
        Ok(output) => println!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_task_events(
    task_id: &str,
    from_seq: u64,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    match api.events(task_id, from_seq).await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_metrics(working_dir: &PathBuf) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    print!("{}", api.metrics());
    Ok(())
}

pub(crate) async fn handle_run_agent_async(
    name: String,
    input: Option<String>,
    version: Option<String>,
    detach: bool,
    working_dir: &PathBuf,
) -> anyhow::Result<()> {
    let registry = Arc::new(archon_core::agents::AgentRegistry::load(working_dir));
    let service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(archon_core::tasks::MetricsRegistry::new());
    let api = archon_core::tasks::CliTaskApi::new(service, metrics);
    match api.submit(name, input, version, detach).await {
        Ok(output) => println!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}
