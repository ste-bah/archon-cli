//! Real activity-delivery await barriers inside the audit adapter.
use super::*;
use archon_workflow::{WorkflowAgentDispatch, WorkflowAgentOutcome, WorkflowUiSink};

struct Delivery {
    nth: usize,
    activities: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl WorkflowUiSink for Delivery {
    async fn emit(&self, event: WorkflowUiEvent) -> archon_workflow::WorkflowUiResult {
        if let WorkflowUiEvent::Activity(update) = event
            && update.status == archon_workflow::WorkflowActivityStatus::Running
            && self.activities.fetch_add(1, Ordering::SeqCst) + 1 == self.nth
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }
}
struct Provider {
    calls: AtomicUsize,
    mode: u8,
}
#[async_trait::async_trait]
impl WorkflowLlmClient for Provider {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("agent port")
    }
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.run_agent(call).await
    }
    async fn run_agent(
        &self,
        _: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 && self.mode == 1 {
            return Err(WorkflowError::StageFailed("connection reset".into()));
        }
        let content = if n == 0 && self.mode == 2 {
            "invalid result".into()
        } else {
            serde_json::to_string(&WorkflowV2Result::accepted("assessed"))?
        };
        Ok(WorkflowAgentOutcome {
            content,
            ..Default::default()
        })
    }
}
async fn activity_takeover(mode: u8) {
    let (_temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let (runner, _rx) = runner(&store, &id, Arc::new(PanicLlm), None, None);
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    let delivery = Arc::new(Delivery {
        nth: if mode == 0 { 1 } else { 2 },
        activities: AtomicUsize::new(0),
        entered: Default::default(),
        release: Default::default(),
    });
    let provider = Arc::new(Provider {
        calls: AtomicUsize::new(0),
        mode,
    });
    let dispatch = AuditDispatch(
        LiveV2AgentClient::new(
            provider.clone(),
            delivery.clone(),
            vec![],
            id.clone(),
            None,
            None,
        )
        .for_audit(),
    );
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "audit-boundary".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input: serde_json::json!({}),
        depends_on: vec![],
    };
    let adapter = WorkflowV2AgentAdapter::new();
    let work = dispatch.run_call(
        "assess",
        None,
        &execution,
        &adapter,
        Some(&runner.v2_store),
        None,
    );
    tokio::pin!(work);
    tokio::select! { biased; result = &mut work => panic!("audit must enter activity delivery: {result:?}"), _ = delivery.entered.notified() => {} }
    let before = provider.calls.load(Ordering::SeqCst);
    assert_eq!(
        before,
        usize::from(mode != 0),
        "reach the named initial/retry/repair delivery"
    );
    take_over(&store, &id);
    delivery.release.notify_one();
    let result = work.await;
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        before,
        "no provider dispatch after takeover in activity delivery"
    );
    assert_stale(&result);
}
#[tokio::test]
async fn round3_291_audit_initial_activity_takeover() {
    activity_takeover(0).await;
}
#[tokio::test]
async fn round3_291_audit_retry_activity_takeover() {
    activity_takeover(1).await;
}
#[tokio::test]
async fn round3_291_audit_repair_activity_takeover() {
    activity_takeover(2).await;
}

#[tokio::test]
async fn round3_291_host_cache_capture_takeover() {
    use archon_workflow::repository_audit::{
        budget::{AuditPolicy, Limit},
        runtime::AuditRuntime,
    };
    let (temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let root = temp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "test"],
        &["config", "user.email", "test@local"],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    std::fs::write(root.join("a.txt"), "source\n").unwrap();
    for args in [&["add", "."][..], &["commit", "-qm", "source"]] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let policy = || AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited,
        total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Unlimited,
    };
    let audit = AuditRuntime::initialize(store.clone(), id.clone(), policy()).unwrap();
    let (mut runner, _rx) = runner(&store, &id, Arc::new(PanicLlm), None, None);
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    runner.runtime.target_repository_root = Some(root.display().to_string());
    runner.client = runner.client.with_audit(audit);
    let host = host_of(runner);
    let taken = Arc::new(std::sync::Mutex::new(None));
    let (other, other_id, capture) = (store.clone(), id.clone(), taken.clone());
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_fixed_persistence::publication_hook::install(store.run_dir(&id), Box::new(move || {
        take_over(&other, &other_id);
        AuditRuntime::initialize(other.clone(), other_id.clone(), policy()).unwrap().update(|state| { state.last_error = Some("successor cache".into()); Ok(()) }).unwrap();
        *capture.lock().unwrap() = Some(snapshot(&other, &other_id));
    }));
    let mut cached = record(&id, "cached");
    cached.call.write_mode = Some(archon_workflow::WorkflowV2WriteMode::Worktree);
    cached.call.options.target_files = vec!["a.txt".into()];
    let out = host.refresh_audit_for_cache(&cached).await;
    assert_stale(&out);
    assert!(
        snapshot(&store, &id) == taken.lock().unwrap().take().expect("capture gap reached"),
        "successor files and bytes unchanged"
    );
}
