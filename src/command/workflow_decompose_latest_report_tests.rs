use super::*;

#[test]
fn fresh_run_without_a_report_resolves_to_none() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "fresh-run".into(),
            task: "fresh run".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();

    let latest = latest_report(&store, &run.id).unwrap();

    assert_eq!(latest, None);
}

#[test]
fn latest_report_follows_event_chain_not_record_order_or_timestamps() {
    let reports = vec![
        ReportRef {
            id: "original".into(),
            supersedes_id: None,
            outcome: "accepted".into(),
            subjects: vec![],
        },
        ReportRef {
            id: "reverify-1-report".into(),
            supersedes_id: Some("original".into()),
            outcome: "needs_remediation".into(),
            subjects: vec!["A".into()],
        },
        ReportRef {
            id: "reverify-2-report".into(),
            supersedes_id: Some("reverify-1-report".into()),
            outcome: "needs_remediation".into(),
            subjects: vec!["A".into()],
        },
        ReportRef {
            id: "resumed-final".into(),
            supersedes_id: None,
            outcome: "accepted".into(),
            subjects: vec![],
        },
    ];
    let events = vec![
        ReportEvent::Reverified {
            report_id: "reverify-1-report".into(),
            supersedes_id: "original".into(),
        },
        ReportEvent::Reverified {
            report_id: "reverify-2-report".into(),
            supersedes_id: "reverify-1-report".into(),
        },
        ReportEvent::Finalized {
            report_id: "resumed-final".into(),
        },
    ];
    let latest = resolve_report_chain(&reports, &events).unwrap().unwrap();
    assert_eq!(latest.id, "resumed-final");
    assert_eq!(latest.outcome, "accepted");
}

#[test]
fn disconnected_reverify_event_is_an_error_instead_of_falling_back_to_old_report() {
    let reports = vec![
        ReportRef {
            id: "original".into(),
            supersedes_id: None,
            outcome: "accepted".into(),
            subjects: vec![],
        },
        ReportRef {
            id: "orphan".into(),
            supersedes_id: Some("other".into()),
            outcome: "needs_remediation".into(),
            subjects: vec![],
        },
    ];
    let events = vec![ReportEvent::Reverified {
        report_id: "orphan".into(),
        supersedes_id: "other".into(),
    }];
    assert!(resolve_report_chain(&reports, &events).is_err());
}
