use super::*;

fn separate_records(status_value: WorkflowV2Status, replay_old: bool, legacy: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (store, id) = fixed_run(&temp, RunStatus::Running);
    let mut old = paused_record(&id, "freeze-acceptance");
    // Lexical order deliberately disagrees with temporal order.
    old.call.id = "z-old-interruption".into();
    old.started_at = "2026-10-01T00:00:00Z".into();
    old.finished_at = "2026-10-01T00:00:01Z".into();
    if legacy {
        old.started_at.clear();
        old.finished_at.clear();
    }
    save(&store, &id, &old);
    project(&store, &id, &old, FixedCallProjectionKind::Interrupted);
    let mut later = host_record(&id);
    later.call.id = "a-new-candidate".into();
    later.call.options.host_command = Some(
        HostCommandRequest::new("freeze-acceptance", Some("different-candidate".into())).unwrap(),
    );
    later.started_at = "2026-10-02T00:00:00Z".into();
    later.finished_at = "2026-10-02T00:00:01Z".into();
    if legacy {
        later.started_at.clear();
        later.finished_at.clear();
    }
    later.status = status_value;
    later.result.status = status_value;
    if status_value == WorkflowV2Status::Failed {
        later.result.data = serde_json::Value::Null;
    }
    save(&store, &id, &later);
    project(&store, &id, &later, FixedCallProjectionKind::Executed);
    if replay_old {
        project(&store, &id, &old, FixedCallProjectionKind::Reused);
    }
    let dispositions = state(&store, &id).dispositions;
    if status_value == WorkflowV2Status::Failed {
        assert_eq!(
            dispositions.get("freeze-acceptance"),
            Some(&SubjectDisposition::Failed)
        );
        assert_ne!(
            dispositions.get("acceptance"),
            Some(&SubjectDisposition::Interrupted)
        );
        assert!(status(&store, &id).contains("freeze-acceptance=failed"));
    } else {
        assert_eq!(
            dispositions.get("acceptance"),
            Some(&SubjectDisposition::Accepted)
        );
        assert!(!dispositions.contains_key("freeze-acceptance"));
        assert!(status(&store, &id).contains("acceptance=accepted"));
    }
    let mut author = later.clone();
    author.call.id = "acceptance-author-2".into();
    author.call.method = WorkflowV2HostMethod::Agent;
    author.call.options.host_command = None;
    author.started_at = "2026-10-03T00:00:00Z".into();
    author.finished_at.clear();
    author.status = WorkflowV2Status::Running;
    author.result.status = WorkflowV2Status::Running;
    save(&store, &id, &author);
    project(&store, &id, &author, FixedCallProjectionKind::Started);
    project(&store, &id, &old, FixedCallProjectionKind::Reused);
    assert_eq!(
        state(&store, &id).dispositions.get("acceptance"),
        Some(&SubjectDisposition::Pending)
    );
    assert!(status(&store, &id).contains("acceptance=pending"));
    if status_value == WorkflowV2Status::Failed {
        assert_eq!(
            state(&store, &id).dispositions.get("freeze-acceptance"),
            Some(&SubjectDisposition::Failed)
        );
    }
}
#[test]
fn review291_old_interruption_cannot_erase_separate_new_failure() {
    for legacy in [false, true] {
        separate_records(WorkflowV2Status::Failed, false, legacy);
    }
}
#[test]
fn review291_old_interruption_cannot_overwrite_separate_new_acceptance() {
    for legacy in [false, true] {
        separate_records(WorkflowV2Status::Accepted, false, legacy);
    }
}
#[test]
fn review291_reprojecting_old_interruption_keeps_new_failure() {
    for legacy in [false, true] {
        separate_records(WorkflowV2Status::Failed, true, legacy);
    }
}
