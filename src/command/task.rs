#![allow(clippy::ptr_arg)]

use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// TASK-AGS-806: /tasks slash-command handler (body-migrate target)
// ---------------------------------------------------------------------------
//
// Real `CommandHandler` impl moved here from the `declare_handler!` stub
// at src/command/registry.rs:330 and the legacy match arm at
// src/command/slash.rs:546-561. Body emits a `TextDelta` (backward-
// compatible user output) plus a `TuiEvent::OpenView(ViewId::Tasks)`
// (forward-compat per AGS-822 — placeholder-handled by run_inner until
// the Stage 7+ overlay ticket lands).
//
// Aliases extended `[todo]` -> `[todo, ps, jobs]` per spec validation
// criterion 4.

use crate::command::registry::{CommandContext, CommandHandler};
use archon_tui::app::TuiEvent;
use archon_tui::events::ViewId;

pub(crate) struct TasksHandler;

impl CommandHandler for TasksHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let tasks = archon_tools::task_manager::TASK_MANAGER.list_tasks();
        let text = if tasks.is_empty() {
            "\nNo background tasks.\n".to_string()
        } else {
            let mut out = format!("\n{} background tasks:\n", tasks.len());
            for t in &tasks {
                out.push_str(&format!("  {} [{}] {}\n", &t.id, t.status, t.description));
            }
            out
        };
        // try_send is the sync analogue of the shipped
        // `let _ = tui_tx.send(...).await` in slash.rs:546-561 — both
        // ignore send failures (channel closed/full). Acceptable here
        // because /tasks output is best-effort UI.
        ctx.emit(TuiEvent::TextDelta(text));
        // AGS-822 forward-compat primitive. View-rendering is
        // placeholder-handled by run_inner until the Stage 7+ overlay
        // ticket lands; the OpenView emission is the contract.
        ctx.emit(TuiEvent::OpenView(ViewId::Tasks));
        Ok(())
    }

    fn description(&self) -> &str {
        "List or manage project tasks"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["todo", "ps", "jobs"]
    }
}

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

// ---------------------------------------------------------------------------
// TASK-AGS-806: tests for /tasks slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::{CommandContext, CommandHandler, default_registry};
    use archon_tui::app::TuiEvent;
    use archon_tui::events::ViewId;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel and
    /// return both the context and the receiver so tests can drain
    /// emitted events.
    fn make_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn tasks_handler_description_matches() {
        let h = TasksHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("task"),
            "TasksHandler description should mention 'task', got: {}",
            h.description()
        );
    }

    #[test]
    fn tasks_handler_aliases_are_todo_ps_jobs() {
        let h = TasksHandler;
        assert_eq!(
            h.aliases(),
            &["todo", "ps", "jobs"],
            "TasksHandler aliases must be [todo, ps, jobs] per AGS-806 + spec validation criterion 4"
        );
    }

    #[test]
    fn tasks_handler_execute_empty_emits_no_background_tasks_text() {
        let (mut ctx, mut rx) = make_ctx();
        let h = TasksHandler;
        h.execute(&mut ctx, &[])
            .expect("TasksHandler::execute must return Ok");

        // Drain channel: collect all events synchronously.
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert!(
            !events.is_empty(),
            "TasksHandler::execute must emit at least one TuiEvent, got 0"
        );
        let saw_text_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
        assert!(
            saw_text_delta,
            "TasksHandler::execute must emit a TuiEvent::TextDelta for empty task list"
        );
    }

    #[test]
    fn tasks_handler_execute_emits_open_view_tasks_event() {
        let (mut ctx, mut rx) = make_ctx();
        let h = TasksHandler;
        h.execute(&mut ctx, &[])
            .expect("TasksHandler::execute must return Ok");

        let mut saw_open_view_tasks = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::OpenView(ViewId::Tasks) = ev {
                saw_open_view_tasks = true;
                break;
            }
        }
        assert!(
            saw_open_view_tasks,
            "TasksHandler::execute must emit TuiEvent::OpenView(ViewId::Tasks) per AGS-822 forward-compat"
        );
    }

    // -----------------------------------------------------------------
    // Registry-level wiring: /tasks aliases ps and jobs must resolve to
    // the same handler as the primary /tasks. Verifies AGS-806 alias
    // extension reaches the public registry surface.
    // -----------------------------------------------------------------

    #[test]
    fn registry_resolves_tasks_aliases_ps_and_jobs() {
        let reg = default_registry();
        let primary = reg.get("tasks").expect("tasks primary must be registered");
        let via_todo = reg
            .get("todo")
            .expect("'todo' alias must resolve to /tasks");
        let via_ps = reg
            .get("ps")
            .expect("'ps' alias must resolve to /tasks per AGS-806");
        let via_jobs = reg
            .get("jobs")
            .expect("'jobs' alias must resolve to /tasks per AGS-806");
        assert_eq!(primary.description(), via_todo.description());
        assert_eq!(primary.description(), via_ps.description());
        assert_eq!(primary.description(), via_jobs.description());
    }
}
