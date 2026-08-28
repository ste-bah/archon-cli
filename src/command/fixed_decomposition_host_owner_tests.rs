use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::fixed_decomposition_host::{
    FixedDecompositionTuiOwner, FixedDecompositionTuiRequest, OwnedExecution,
    deliver_terminal_with_timeout, spawn,
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
fn retained_owner_record_requires_one_identity_and_accumulates_closed_actions() {
    let project = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::project(project.path());
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "owner-record".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let owner = FixedDecompositionTuiOwner::default();
    crate::command::workflow_decompose_owner::initialize(
        &store,
        &run.id,
        owner.owner_identity.as_str(),
    )
    .unwrap();
    assert!(
        crate::command::workflow_decompose_owner::require_owner(
            &store,
            &run.id,
            Some("different-owner-identity"),
        )
        .is_err()
    );
    assert!(
        crate::command::workflow_decompose_owner::require_owner(&store, &run.id, None).is_err()
    );
    for action in ["pause", "status", "resume", "cancel"] {
        crate::command::workflow_decompose_owner::record_action(
            &store,
            &run.id,
            Some(owner.owner_identity.as_str()),
            action,
        )
        .unwrap();
    }
    let record = crate::command::workflow_decompose_owner::read(&store, &run.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        record.actions,
        std::collections::BTreeSet::from([
            "cancel".to_string(),
            "launch".to_string(),
            "pause".to_string(),
            "resume".to_string(),
            "status".to_string(),
        ])
    );
}

#[tokio::test]
async fn shutdown_joins_worker_even_when_run_id_lock_is_poisoned() {
    let owner = FixedDecompositionTuiOwner::default();
    let run_id = Arc::new(Mutex::new(Some("wf-poisoned-owner".to_string())));
    let poison = Arc::clone(&run_id);
    let _ = std::thread::spawn(move || {
        let _guard = poison.lock().unwrap();
        panic!("poison run-id lock");
    })
    .join();
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancellation_for_worker = Arc::clone(&cancellation);
    let joined = Arc::new(AtomicBool::new(false));
    let joined_for_worker = Arc::clone(&joined);
    let handle = tokio::spawn(async move {
        while !cancellation_for_worker.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        joined_for_worker.store(true, Ordering::SeqCst);
    });
    *owner.inner.lock().unwrap() = Some(OwnedExecution {
        project_root: std::env::temp_dir(),
        run_id,
        cancellation_requested: cancellation,
        handle,
    });

    let error = owner.cancel_and_wait().await.unwrap_err();

    assert!(error.to_string().contains("run-id owner lock is poisoned"));
    assert!(joined.load(Ordering::SeqCst));
}

#[tokio::test]
async fn saturated_terminal_delivery_writes_durable_deferral_marker() {
    let project = tempfile::tempdir().unwrap();
    let task_root = project.path().join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    let task_root = task_root.canonicalize().unwrap();
    let store = archon_workflow::WorkflowStore::project(project.path());
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "terminal-delivery".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let identity = archon_workflow::FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "rev".into(),
        script_digest: "a".repeat(64),
        catalog_digest: "b".repeat(64),
        project_root_identity: project.path().canonicalize().unwrap().display().to_string(),
        prd_identity: project.path().join("PRD.md").display().to_string(),
        task_root_identity: task_root.display().to_string(),
    };
    let log_path = task_root.join(".decompose.log");
    store
        .write_run_json(
            &run.id,
            crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH,
            &archon_workflow::FixedDecompositionStateV1 {
                schema_version: 1,
                run_kind: archon_workflow::WorkflowRunKind::FixedDecompositionV1,
                identity,
                phase: archon_workflow::DecompositionPhase::Identity,
                attempts: Default::default(),
                dispositions: Default::default(),
                log_path: log_path.display().to_string(),
            },
        )
        .unwrap();
    let (tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    tx.send(archon_tui::app::TuiEvent::GenerationStarted)
        .unwrap();
    let sink = archon_workflow::ui_sink_port::ResilientWorkflowUiSink::wrap(
        crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink::arc(tx),
    );
    let root = project.path().to_path_buf();
    let run_id = run.id.clone();
    let slot = Arc::new(Mutex::new(Some(run.id)));
    let slot_for_task = Arc::clone(&slot);
    let task = tokio::spawn(async move {
        deliver_terminal_with_timeout(
            &sink,
            archon_workflow::WorkflowUiEvent::Error("terminal".into()),
            &root,
            slot_for_task.as_ref(),
            std::time::Duration::from_millis(20),
        )
        .await
    });
    let error = task.await.unwrap().unwrap_err();

    assert!(error.to_string().contains("exceeded its bounded wait"));
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(log.contains("event=ui_terminal_delivery_deferred"), "{log}");
    assert!(log.contains(&format!("run_id={run_id}")), "{log}");
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

#[test]
fn fixed_worker_completion_uses_bounded_backpressured_terminal_sink() {
    let source = include_str!("fixed_decomposition_host.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("production source");
    assert!(source.contains("FixedDecompositionWorkflowUiSink::arc"));
    assert!(!source.contains("ResilientWorkflowUiSink::wrap"));
    assert!(source.contains("TuiWorkflowUiSink::arc"));
    assert!(source.contains("deliver_terminal("));
    assert!(source.contains("deliver_terminal_with_timeout("));
    assert!(source.contains("ui_terminal_delivery_deferred"));
}
