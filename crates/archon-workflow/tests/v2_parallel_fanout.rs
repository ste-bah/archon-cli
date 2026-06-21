use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use archon_workflow::{
    WorkflowError, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2Scheduler, WorkflowV2SchedulerConfig,
    WorkflowV2Status, WorkflowV2WriteMode,
};

fn item(id: &str, role: &str) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem::read_only(
        id,
        role,
        WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        serde_json::json!({ "item": id }),
    )
}

fn accepted(summary: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted(summary);
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "branch inspected concrete input",
    ));
    result
}

fn failed(summary: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: summary.to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "branch failure evidence",
        )],
        artifacts: Vec::new(),
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test focused_branch".to_string(),
            status: WorkflowV2CommandStatus::Failed,
            exit_code: Some(101),
            output_summary: "focused branch failed".to_string(),
        }],
        files_read: Vec::new(),
        files_changed: Vec::new(),
        task_coverage: Vec::new(),
        residual_gaps: Vec::new(),
        data: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn read_only_fanout_runs_branches_concurrently() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism: 4,
        absolute_max_parallelism: 8,
        role_limits: BTreeMap::new(),
    });
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let items = (0..4)
        .map(|idx| item(&format!("inspect-{idx}"), "reader"))
        .collect::<Vec<_>>();

    let report = scheduler
        .run_read_only_fanout(items, |branch| {
            let active = active.clone();
            let peak = peak.clone();
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(25)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert_eq!(report.outcomes.len(), 4);
    assert!(peak.load(Ordering::SeqCst) > 1);
    assert!(report.peak_parallelism > 1);
}

#[tokio::test]
async fn read_only_fanout_observer_receives_each_branch_outcome() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism: 2,
        absolute_max_parallelism: 8,
        role_limits: BTreeMap::new(),
    });
    let observed = Arc::new(AtomicUsize::new(0));
    let items = vec![item("inspect-a", "reader"), item("inspect-b", "reader")];

    let report = scheduler
        .run_read_only_fanout_observed(
            items,
            {
                let observed = observed.clone();
                move |outcome| {
                    assert_eq!(outcome.status, WorkflowV2Status::Accepted);
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            |branch| async move { Ok(accepted(&format!("{} accepted", branch.id))) },
        )
        .await
        .expect("fanout");

    assert_eq!(report.outcomes.len(), 2);
    assert_eq!(observed.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn read_only_fanout_propagates_control_pause_instead_of_branch_failure() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());

    let err = scheduler
        .run_read_only_fanout(vec![item("inspect-a", "reader")], |_branch| async {
            Err(WorkflowError::ControlPaused("generation 2".to_string()))
        })
        .await
        .expect_err("control pause should propagate");

    assert!(matches!(err, WorkflowError::ControlPaused(_)));
}

#[tokio::test]
async fn empty_item_list_without_noop_proof_fails_early() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());

    let err = scheduler
        .run_read_only_fanout(Vec::new(), |_branch| async {
            Ok(accepted("branch should not run"))
        })
        .await
        .expect_err("empty fanout must fail before branch execution");

    assert!(
        err.to_string()
            .contains("zero items without typed no-op proof")
    );
}

#[tokio::test]
async fn global_cap_is_enforced() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism: 2,
        absolute_max_parallelism: 8,
        role_limits: BTreeMap::new(),
    });
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let items = (0..6)
        .map(|idx| item(&format!("inspect-{idx}"), "reader"))
        .collect::<Vec<_>>();

    let report = scheduler
        .run_read_only_fanout(items, |branch| {
            let active = active.clone();
            let peak = peak.clone();
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert_eq!(report.max_parallelism, 2);
    assert!(peak.load(Ordering::SeqCst) <= 2);
    assert!(report.peak_parallelism <= 2);
}

#[tokio::test]
async fn absolute_cap_clamps_requested_parallelism() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism: 8,
        absolute_max_parallelism: 2,
        role_limits: BTreeMap::new(),
    });
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let items = (0..5)
        .map(|idx| item(&format!("inspect-{idx}"), "reader"))
        .collect::<Vec<_>>();

    let report = scheduler
        .run_read_only_fanout(items, |branch| {
            let active = active.clone();
            let peak = peak.clone();
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert_eq!(report.max_parallelism, 2);
    assert!(peak.load(Ordering::SeqCst) <= 2);
}

#[tokio::test]
async fn per_role_cap_is_enforced() {
    let mut role_limits = BTreeMap::new();
    role_limits.insert("reviewer".to_string(), 1);
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism: 4,
        absolute_max_parallelism: 8,
        role_limits,
    });
    let active_reviewers = Arc::new(AtomicUsize::new(0));
    let peak_reviewers = Arc::new(AtomicUsize::new(0));
    let items = (0..4)
        .map(|idx| item(&format!("review-{idx}"), "reviewer"))
        .collect::<Vec<_>>();

    scheduler
        .run_read_only_fanout(items, |branch| {
            let active_reviewers = active_reviewers.clone();
            let peak_reviewers = peak_reviewers.clone();
            async move {
                let now = active_reviewers.fetch_add(1, Ordering::SeqCst) + 1;
                peak_reviewers.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active_reviewers.fetch_sub(1, Ordering::SeqCst);
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert_eq!(peak_reviewers.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn branch_failure_is_captured_without_losing_successful_evidence() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    let items = vec![
        item("a", "reader"),
        item("b", "reader"),
        item("c", "reader"),
    ];

    let report = scheduler
        .run_read_only_fanout(items, |branch| async move {
            if branch.id == "b" {
                Ok(failed("branch b failed"))
            } else {
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert_eq!(report.outcomes.len(), 3);
    assert_eq!(report.typed_results().len(), 3);
    assert_eq!(report.failed_outcomes().len(), 1);
    assert!(
        report.outcomes.iter().any(|outcome| {
            outcome.item_id == "a" && outcome.status == WorkflowV2Status::Accepted
        })
    );
    assert!(
        report.outcomes.iter().any(|outcome| {
            outcome.item_id == "c" && outcome.status == WorkflowV2Status::Accepted
        })
    );
}

#[tokio::test]
async fn handler_error_is_captured_without_stopping_siblings() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    let items = vec![
        item("a", "reader"),
        item("b", "reader"),
        item("c", "reader"),
    ];

    let report = scheduler
        .run_read_only_fanout(items, |branch| async move {
            if branch.id == "b" {
                Err(WorkflowError::StageFailed("branch transport failed".into()))
            } else {
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert_eq!(report.outcomes.len(), 3);
    assert_eq!(report.failed_outcomes().len(), 1);
    assert!(report.outcomes.iter().any(|outcome| {
        outcome.item_id == "b"
            && outcome.status == WorkflowV2Status::Failed
            && outcome
                .error
                .as_deref()
                .is_some_and(|error| error.contains("branch transport failed"))
    }));
    assert_eq!(report.typed_results().len(), 2);
}

#[tokio::test]
async fn write_capable_branch_is_rejected_by_read_only_scheduler() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    let mut write_item = item("write-a", "coder");
    write_item.call.write_mode = Some(WorkflowV2WriteMode::Coordinated);

    let err = scheduler
        .run_read_only_fanout(vec![write_item], |_| async {
            Ok(accepted("should not run"))
        })
        .await
        .expect_err("write fanout should be rejected");

    assert!(matches!(err, WorkflowError::PolicyDenied(_)));
}

#[tokio::test]
async fn cancellation_stops_pending_branches() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism: 1,
        absolute_max_parallelism: 8,
        role_limits: BTreeMap::new(),
    });
    let token = scheduler.cancellation_token();
    let items = vec![
        item("first", "reader"),
        item("pending-a", "reader"),
        item("pending-b", "reader"),
    ];

    let report = scheduler
        .run_read_only_fanout(items, |branch| {
            let token = token.clone();
            async move {
                if branch.id == "first" {
                    token.cancel();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
                Ok(accepted(&format!("{} accepted", branch.id)))
            }
        })
        .await
        .expect("fanout");

    assert!(report.cancelled);
    assert_eq!(
        report
            .outcomes
            .iter()
            .filter(|outcome| outcome.status == WorkflowV2Status::Cancelled)
            .count(),
        2
    );
}
