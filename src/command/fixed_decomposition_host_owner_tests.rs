use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::fixed_decomposition_host::{
    FixedDecompositionTuiOwner, FixedDecompositionTuiRequest, OwnedExecution, spawn,
};

#[tokio::test]
async fn retained_owner_refuses_a_second_active_decomposition() {
    let owner = FixedDecompositionTuiOwner::default();
    let first = tokio::spawn(std::future::pending::<()>());
    *owner.inner.lock().unwrap() = Some(OwnedExecution {
        project_root: std::env::temp_dir(),
        run_id: Arc::new(Mutex::new(None)),
        cancellation_requested: Arc::new(AtomicBool::new(false)),
        handle: first,
    });
    let (tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel();

    let error = spawn(
        std::env::temp_dir(),
        FixedDecompositionTuiRequest {
            prd_path: "PRD-X.md".into(),
            task_root: "tasks/PRD-X".into(),
        },
        archon_core::config::ArchonConfig::default(),
        archon_core::env_vars::load_env_vars_from(&Default::default()),
        tx,
        owner.clone(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("already active"), "{error}");
    owner.inner.lock().unwrap().take().unwrap().handle.abort();
}

#[tokio::test]
async fn shutdown_requests_cancellation_before_run_id_is_persisted() {
    let owner = FixedDecompositionTuiOwner::default();
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancellation_for_worker = Arc::clone(&cancellation);
    let handle = tokio::spawn(async move {
        while !cancellation_for_worker.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    });
    *owner.inner.lock().unwrap() = Some(OwnedExecution {
        project_root: std::env::temp_dir(),
        run_id: Arc::new(Mutex::new(None)),
        cancellation_requested: Arc::clone(&cancellation),
        handle,
    });

    owner.cancel_and_wait().await.unwrap();

    assert!(cancellation.load(Ordering::SeqCst));
    assert!(owner.inner.lock().unwrap().is_none());
}

#[tokio::test]
async fn shutdown_cancels_persisted_run_and_waits_for_worker() {
    let project = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::project(project.path());
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "owner-test".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = archon_workflow::RunStatus::Running;
    store.save_state(&run).unwrap();
    let run_id = run.id.clone();
    let store_for_worker = store.clone();
    let run_for_worker = run.id.clone();
    let handle = tokio::spawn(async move {
        loop {
            if store_for_worker.load_state(&run_for_worker).unwrap().status
                == archon_workflow::RunStatus::Cancelled
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    });
    let owner = FixedDecompositionTuiOwner::default();
    *owner.inner.lock().unwrap() = Some(OwnedExecution {
        project_root: project.path().to_path_buf(),
        run_id: Arc::new(Mutex::new(Some(run_id.clone()))),
        cancellation_requested: Arc::new(AtomicBool::new(false)),
        handle,
    });

    owner.cancel_and_wait().await.unwrap();

    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Cancelled
    );
    assert!(owner.inner.lock().unwrap().is_none());
}

#[test]
fn production_spawn_retains_the_worker_handle() {
    let source = include_str!("fixed_decomposition_host.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("production source");
    assert!(source.contains("*guard = Some(OwnedExecution {"));
    assert!(source.contains("handle,"));
}

#[test]
fn session_shutdown_cancels_and_awaits_the_retained_worker() {
    let source = include_str!("../session_loop/mod.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("production source");
    assert!(source.contains("fixed_decomposition_owner.cancel_and_wait().await"));
}
