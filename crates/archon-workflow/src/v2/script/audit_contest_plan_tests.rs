//! Issue-112b: the host's contest plan and its dispatch check.

use super::*;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostOptions};

#[test]
fn a_confirmation_id_is_deterministic_and_names_the_declarer() {
    let id = confirmation_id(
        "TASK-TRADING-002",
        "registry-migration-report.json",
        "absent",
    );
    assert_eq!(
        id,
        confirmation_id(
            "TASK-TRADING-002",
            "registry-migration-report.json",
            "absent"
        )
    );
    assert!(id.starts_with("audit-confirm-task-trading-002-"), "{id}");
    assert_ne!(
        id,
        confirmation_id(
            "TASK-TRADING-002",
            "registry-migration-report.json",
            "present"
        )
    );
}

fn confirmation(extra: serde_json::Value) -> WorkflowV2CallExecution {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(AUDIT_CONTEST_OPTION.into(), extra);
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "verification-wave-audit-confirm-x".into(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        input: json!({"source_data": [{"canonical_task_ids": ["TASK-X"]}]}),
        depends_on: vec![],
    }
}

/// With no contest the host knows of, a confirmation is never answered;
/// an ordinary verifier is never judged here.
#[test]
fn a_confirmation_without_a_host_contest_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let call = confirmation(json!({"path": "r.json", "state": "absent", "declarer": "TASK-X"}));
    let why = confirmation_refusal(&call, &store, Some(temp.path())).expect("refused");
    assert!(why.contains("no contest"), "{why}");
    let mut plain = call.clone();
    plain.call.options.extra.clear();
    assert_eq!(
        confirmation_refusal(&plain, &store, Some(temp.path())),
        None
    );
    assert!(contest_plan(&store, Some(temp.path())).is_empty());
}
