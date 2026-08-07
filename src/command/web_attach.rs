//! The web dashboard running INSIDE the TUI process.
//!
//! `archon web` is a second process: it opens its own stores and builds its own
//! chat backend, so it shows its own session rather than the one you are
//! sitting in. Worse, the two registries that know what agents are doing —
//! `BACKGROUND_AGENTS` and `TASK_MANAGER` — hold live `JoinHandle`s and
//! `CancellationToken`s, so no amount of serialisation would let a separate
//! process read them. The only way to observe the running session is to be in
//! it, which is what this module does: `tokio::spawn` the axum server in the
//! TUI's own runtime, handing it the handles the TUI already holds.
//!
//! The server is tracked in a process-global slot because the entry point is a
//! sync slash handler and the teardown is in session shutdown — two places that
//! share no context. One TUI process hosts at most one attached server, so a
//! single slot is the whole state space.

use std::sync::{Arc, Mutex, OnceLock};

use archon_sdk::web::{
    WebConfig, WebPolicySummary, WebRuntimeHandles, WebRuntimePaths, WebServer,
    WebSubsystemPolicySummary, api::EffectivePolicySummary,
};

/// How long session teardown waits for the server task to wind down before
/// giving up. The TUI is already exiting; a hung server must not hold it.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct AttachedServer {
    url: String,
    shutdown: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

fn slot() -> &'static Mutex<Option<AttachedServer>> {
    static SLOT: OnceLock<Mutex<Option<AttachedServer>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// What the TUI hands the attached server: its own open state.
pub(crate) struct AttachOptions {
    pub(crate) port: u16,
    pub(crate) working_dir: std::path::PathBuf,
    pub(crate) memory: Option<Arc<dyn archon_memory::MemoryTrait>>,
}

/// Start the attached server, returning its URL.
///
/// Sync so it can be called from a `CommandHandler`; the spawn needs a tokio
/// runtime, which the slash dispatch path is always inside.
pub(crate) fn start(options: AttachOptions) -> anyhow::Result<String> {
    let mut guard = slot().lock().expect("attached web slot poisoned");
    if let Some(existing) = guard.as_ref() {
        anyhow::bail!("web dashboard already running at {}", existing.url);
    }

    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| anyhow::anyhow!("web dashboard needs a running tokio runtime"))?;

    let config = WebConfig {
        port: options.port,
        bind_address: "127.0.0.1".to_string(),
        // The TUI owns the terminal and the browser is a separate decision;
        // the handler prints the URL instead.
        open_browser: false,
        ..WebConfig::default()
    };
    let url = format!("http://{}:{}", config.bind_address, config.port);

    let mut paths = WebRuntimePaths::from_overrides(None, None);
    // Project-scoped views (workflows, corpus) resolve against the session's
    // working directory, not wherever the process happened to start.
    paths.cwd = options.working_dir;

    let handles = WebRuntimeHandles {
        live: None,
        memory: options.memory,
    };
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server_shutdown = shutdown.clone();
    // Loopback only, so no bearer token: the same posture `archon web` takes
    // for a localhost bind.
    let server = WebServer::attached(config, None, attached_policy(&paths.cwd), paths, handles);
    let task = handle.spawn(async move {
        let shutdown = server_shutdown.clone();
        if let Err(error) = server
            .run_until(async move { shutdown.cancelled().await })
            .await
        {
            tracing::warn!(%error, "attached web dashboard stopped");
        }
    });

    *guard = Some(AttachedServer {
        url: url.clone(),
        shutdown,
        task,
    });
    Ok(url)
}

/// URL of the running attached server, if any.
pub(crate) fn running_url() -> Option<String> {
    slot()
        .lock()
        .expect("attached web slot poisoned")
        .as_ref()
        .map(|server| server.url.clone())
}

/// Signal the server to stop without waiting. Safe from sync contexts.
///
/// The slot is freed immediately so `/web` can start a fresh server; the task
/// is reaped in the background rather than blocking the input loop on a
/// graceful shutdown that may take seconds.
pub(crate) fn request_stop() -> Option<String> {
    let server = slot().lock().expect("attached web slot poisoned").take()?;
    server.shutdown.cancel();
    let url = server.url.clone();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, server.task).await;
        });
    }
    Some(url)
}

/// Stop the attached server and wait for its task, bounded by
/// [`SHUTDOWN_TIMEOUT`]. Called from session teardown so the server cannot
/// outlive the TUI that spawned it.
pub(crate) async fn shutdown() {
    let Some(server) = slot().lock().expect("attached web slot poisoned").take() else {
        return;
    };
    server.shutdown.cancel();
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, server.task).await {
        Ok(Ok(())) => tracing::info!("attached web dashboard shut down"),
        Ok(Err(error)) => tracing::warn!(%error, "attached web dashboard task failed"),
        Err(_) => tracing::warn!("attached web dashboard did not stop within {SHUTDOWN_TIMEOUT:?}"),
    }
}

/// Policy for the attached server, read from the session's working directory
/// rather than the process cwd.
fn attached_policy(cwd: &std::path::Path) -> EffectivePolicySummary {
    let policy = archon_policy::load_effective_policy(cwd).unwrap_or_default();
    EffectivePolicySummary {
        web: WebPolicySummary {
            allow_mutating_actions: policy.web.allow_mutating_actions,
            allow_file_uploads: policy.web.allow_file_uploads,
            allow_pipeline_controls: policy.web.allow_pipeline_controls,
            allow_model_training_actions: policy.web.allow_model_training_actions,
            allow_corpus_open_paths: policy.web.allow_corpus_open_paths,
        },
        subsystem: WebSubsystemPolicySummary {
            allow_behavior_proposal_actions: true,
            allow_model_behavior_changes: policy.world_model.allow_behavior_changes,
            allow_pipeline_controls: policy.web.allow_pipeline_controls,
            allow_corpus_open_paths: policy.web.allow_corpus_open_paths,
            allow_file_uploads: policy.web.allow_file_uploads,
        },
        action_gate: "global web mutation gate AND action-family gate AND subsystem gate"
            .to_string(),
        requires_confirmation: vec![
            "pipeline control".to_string(),
            "model promotion".to_string(),
            "training action".to_string(),
            "corpus filesystem open".to_string(),
            "behaviour proposal approval".to_string(),
        ],
    }
}

#[cfg(test)]
#[path = "web_attach_tests.rs"]
mod tests;
