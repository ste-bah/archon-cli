//! Issue 358 round 3: the visible copies of a transition match only that
//! transition, survive a torn log line, and a transition that no executor
//! ever ran on says so.
use super::*;

#[tokio::test]
async fn upgrade_358_lost_record_never_adopts_another_transitions_event() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path())
            .await;
    let (message, _) = resume_as(project.path(), &store, &run_id, "rev-b").await;
    assert!(message.contains("barrier observed"), "{message}");
    let first = transitions(&store, &run_id).transitions[0]
        .event_id
        .unwrap();
    // The record is lost (a crash before its rename reached the disk); the
    // next resume, on another build, records index 0 again.
    std::fs::remove_file(store.run_dir(&run_id).join(TRANSITIONS_PATH)).unwrap();
    let (message, printed) = resume_as(project.path(), &store, &run_id, "rev-c").await;
    assert!(message.contains("barrier observed"), "{message}");
    let record = transitions(&store, &run_id);
    assert_eq!(record.transitions.len(), 1);
    assert_eq!(record.transitions[0].new.starting_binary_revision, "rev-c");
    let seq = record.transitions[0].event_id.unwrap();
    assert_ne!(seq, first, "the rev-b event is not the rev-c transition");
    let shown = transition_events(&store, &run_id);
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert_eq!(
        shown[1]["detail"]["new"]["starting_binary_revision"],
        "rev-c"
    );
    assert!(
        log_of(&store, &run_id).contains(&format!(
            "event_id={seq} transition=binary_revision_drift persisted="
        )) && log_of(&store, &run_id).contains("current=rev-c"),
        "{}",
        log_of(&store, &run_id)
    );
    assert!(
        printed.iter().any(|line| line.contains("current=rev-c")),
        "{printed:?}"
    );
}

#[tokio::test]
async fn upgrade_358_torn_log_tail_gets_its_own_line_once() {
    for tail in ["torn line without an end", "event_id=1 transition=x"] {
        let project = fixture_project();
        let (store, run_id, _) =
            super::super::super::workflow_decomposition_drift_tests::launch_and_pause(
                project.path(),
            )
            .await;
        let state: FixedDecompositionStateV1 =
            read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
        let mut log = std::fs::read_to_string(&state.log_path).unwrap();
        log.push_str(tail);
        std::fs::write(&state.log_path, &log).unwrap();
        for _ in 0..3 {
            let (message, _) = resume_as(project.path(), &store, &run_id, "rev-b").await;
            assert!(message.contains("barrier observed"), "{message}");
        }
        let seq = transitions(&store, &run_id).transitions[0]
            .event_id
            .unwrap();
        let log = log_of(&store, &run_id);
        let key = format!("event_id={seq} transition=binary_revision_drift ");
        assert_eq!(log.matches(&key).count(), 1, "{tail}: {log}");
        assert!(
            log.lines().any(|line| line.starts_with(&key)),
            "{tail}: {log}"
        );
        assert!(log.contains(&format!("{tail}\n")), "{tail}: {log}");
    }
}

#[tokio::test]
async fn upgrade_358_transition_is_started_only_by_an_executor() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path())
            .await;
    // Refused at provider construction: admitted, never executed.
    let (message, _) = resume_as(project.path(), &store, &run_id, "rev-b").await;
    assert!(message.contains("barrier observed"), "{message}");
    assert_eq!(transitions(&store, &run_id).transitions[0].started_at, None);
    let status = crate::command::workflow_decompose_transitions::status_lines(&store, &run_id);
    assert!(
        status.contains("binary_revision=rev-b") && status.contains("started=never"),
        "{status}"
    );
    // Every check passes and an executor takes the run on this runtime.
    let _ = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &ReadyFactory,
        Arc::new(Lines(Arc::new(Mutex::new(Vec::new())))),
        None,
        None,
        "rev-b",
    )
    .await;
    let record = transitions(&store, &run_id);
    assert_eq!(
        record.transitions.len(),
        1,
        "same runtime: no new transition"
    );
    let started = record.transitions[0].started_at.clone().expect("started");
    let status = crate::command::workflow_decompose_transitions::status_lines(&store, &run_id);
    assert!(status.contains(&format!("started={started}")), "{status}");
}
