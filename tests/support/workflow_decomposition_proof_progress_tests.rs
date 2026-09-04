use super::*;

fn event(seq: u64, kind: &str, detail: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"seq": seq, "kind": kind, "detail": detail})
}

fn terminal(seq: u64, status: &str) -> serde_json::Value {
    event(
        seq,
        "stage_stalled",
        serde_json::json!({"event": "terminal_status", "status": status}),
    )
}

/// The awaited event wins; a genuinely terminal status without it is a definite
/// failure; a `terminal_status` label carrying `paused` or `running` is not.
#[test]
fn event_wait_verdict_distinguishes_awaited_ended_pending_and_paused() {
    let markers = ["skeleton-author-1"];
    let kind = "author_attempt_started";
    let pending = vec![event(
        1,
        "stage_started",
        serde_json::json!({"call_id": "acceptance-author-1"}),
    )];
    assert!(event_wait_verdict(&pending, kind, &markers).is_none());
    let awaited = vec![event(
        2,
        kind,
        serde_json::json!({"call_id": "skeleton-author-1"}),
    )];
    assert_eq!(event_wait_verdict(&awaited, kind, &markers), Some(Ok(())));
    let ended = vec![terminal(3, "needs_review")];
    let error = event_wait_verdict(&ended, kind, &markers)
        .unwrap()
        .unwrap_err();
    assert!(error.contains("terminal status needs_review"), "{error}");
    for not_terminal in ["paused", "running", "planned"] {
        let labelled = vec![terminal(4, not_terminal)];
        assert!(
            event_wait_verdict(&labelled, kind, &markers).is_none(),
            "{not_terminal} must keep the wait alive"
        );
    }
}

/// The terminal whitelist must name real run statuses, spelled as the engine
/// serialises them, or a renamed status would silently never end a wait.
#[test]
fn terminal_statuses_are_real_run_statuses() {
    for status in TERMINAL_STATUSES {
        let parsed: Result<archon_workflow::RunStatus, _> =
            serde_json::from_value(serde_json::json!(status));
        assert!(parsed.is_ok(), "{status} is not a RunStatus");
    }
}

/// A run that keeps appending events outlives a short idle window; a silent run
/// does not; the cap still bounds an endlessly busy run.
#[test]
fn progressing_wait_restarts_its_clock_on_new_events() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().to_path_buf();
    let path = archon_workflow::WorkflowStore::project(&project).events_path("wf-progress");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&writer_path)
            .unwrap();
        for seq in 1..=12u64 {
            std::thread::sleep(Duration::from_millis(200));
            let kind = if seq == 12 {
                "author_attempt_started"
            } else {
                "stage_started"
            };
            let line = event(
                seq,
                kind,
                serde_json::json!({"call_id": "skeleton-author-1"}),
            );
            writeln!(file, "{line}").unwrap();
        }
    });
    let markers = ["author_attempt_started", "skeleton-author-1"];
    wait_for_event_line_while_progressing(
        &project,
        "wf-progress",
        &markers,
        Duration::from_secs(2),
        Duration::from_secs(60),
    )
    .expect("twelve events 200ms apart outlive a 2s idle window");
    writer.join().unwrap();

    let silent = wait_for_event_line_while_progressing(
        &project,
        "wf-silent",
        &markers,
        Duration::from_millis(300),
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(silent.contains("no progress for 0s"), "{silent}");

    let capped = wait_for_event_line_while_progressing(
        &project,
        "wf-silent",
        &markers,
        Duration::from_secs(30),
        Duration::from_millis(300),
    )
    .unwrap_err();
    assert!(capped.contains("exceeded the 0s cap"), "{capped}");
}

/// A persistently unreadable event log is named in the failure instead of
/// masquerading as an idle run.
#[test]
fn unreadable_event_log_is_named_in_the_failure() {
    let temp = tempfile::tempdir().unwrap();
    let path = archon_workflow::WorkflowStore::project(temp.path()).events_path("wf-bad");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json\n").unwrap();
    let error = wait_for_event_line_while_progressing(
        temp.path(),
        "wf-bad",
        &["author_attempt_started", "x"],
        Duration::from_millis(300),
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(error.contains("event log unreadable"), "{error}");
}
