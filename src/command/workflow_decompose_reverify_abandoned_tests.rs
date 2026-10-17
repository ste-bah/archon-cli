use super::*;

#[tokio::test]
async fn abandoned_started_invocation_with_gate_records_is_ignored_on_next_reverify() {
    let fixture = fixture();
    let saved = std::sync::Arc::new(std::sync::Mutex::new(false));
    let saved_hook = saved.clone();
    crate::command::workflow::workflow_decompose_reverify::persist::set_after_persist_hook(
        move || {
            *saved_hook.lock().unwrap() = true;
            true
        },
    );
    let stopped = crate::command::workflow::workflow_decompose_reverify::reverify(
        &fixture.project,
        &fixture.run_id,
        &archon_core::config::ArchonConfig::default(),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
    )
    .await;
    assert!(stopped.unwrap_err().to_string().contains("test crash seam"));
    assert!(*saved.lock().unwrap());

    let (events, damaged) =
        archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
            .unwrap();
    assert_eq!(damaged, 0);
    let invocation = events
        .iter()
        .find(|event| {
            event.kind == archon_workflow::WorkflowEventKind::DecompositionReverifyStarted
        })
        .unwrap()
        .detail["invocation_id"]
        .as_str()
        .unwrap()
        .to_string();
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    assert!(
        v2.load_call_records()
            .unwrap()
            .iter()
            .any(|record| { record.call.id.starts_with(&format!("{invocation}-")) })
    );
    assert!(!events.iter().any(|event| {
        event.kind == archon_workflow::WorkflowEventKind::DecompositionReverified
            && event.detail["invocation_id"] == invocation
    }));

    let next = invoke_reverify(&archon_binary(), &fixture);
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_ne!(latest.id, format!("{invocation}-report"));
    assert!(
        String::from_utf8_lossy(&next.stderr).contains("reverify gate started"),
        "next invocation was admitted but did not execute gates: {}",
        String::from_utf8_lossy(&next.stderr)
    );
}
