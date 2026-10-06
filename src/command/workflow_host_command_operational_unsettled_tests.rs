//! Issue 338: a child that read nothing over a task-set publish no read
//! could settle ends with the documented exit and evidence line, and the
//! executor pauses the run on it at once -- never a completion, a retry or a
//! failure.
use super::*;

const EVIDENCE: &str = "an interrupted publish of this task set left its journal /p.publish-journal (state: committed) and it could not be settled: log. Operator remedy: fix it";

#[test]
fn only_the_unsettled_exit_with_its_line_is_an_unsettled_publish() {
    let line = unsettled_publish_line(&format!("{EVIDENCE}\nsecond line"));
    assert!(!line.contains('\n'), "{line}");
    let stderr = format!("noise\n{}\n{line}\n", progress_line(2));
    let ended = output(Some(EXIT_UNSETTLED_PUBLISH), false, &stderr);
    assert_eq!(classify(&ended), Some(OperationalKind::UnsettledPublish));
    let evidence = unsettled_publish_evidence(ended.exit_code, &ended.stderr).unwrap();
    assert!(evidence.starts_with("an interrupted publish"), "{evidence}");
    // The exit alone is the command's own status; so is the line alone.
    for (code, stderr) in [
        (Some(EXIT_UNSETTLED_PUBLISH), "no marker".to_string()),
        (
            Some(EXIT_UNSETTLED_PUBLISH),
            format!("{UNSETTLED_PUBLISH_MARKER}  "),
        ),
        (Some(1), line.clone()),
        (None, line.clone()),
    ] {
        assert_eq!(
            classify(&output(code, false, &stderr)),
            None,
            "{code:?} {stderr}"
        );
        assert_eq!(unsettled_publish_evidence(code, stderr.as_bytes()), None);
    }
}

#[tokio::test]
async fn an_unsettled_publish_exit_pauses_the_run_at_once_with_the_childs_evidence() {
    let line = unsettled_publish_line(EVIDENCE);
    let fixture = fixture(vec![Scripted::Stderr(EXIT_UNSETTLED_PUBLISH, line)]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1, "retried");
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("an unsettled publish pauses, never fails: {error:?}");
    };
    for needed in [
        "'task-set-lint'",
        "state: committed",
        "Operator remedy",
        &format!("archon workflow resume --live --yes {}", fixture.run_id),
    ] {
        assert!(message.contains(needed), "{needed} missing: {message}");
    }
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.stages["host-call"].status, StageStatus::Paused);
    let events = events(&fixture);
    let paused = events_named(&events, "host_command_unsettled_publish");
    assert_eq!(paused.len(), 1, "{events:?}");
    assert_eq!(paused[0]["kind"], "paused");
    assert!(
        paused[0]["detail"]["evidence"]
            .as_str()
            .unwrap()
            .contains("state: committed")
    );
    assert!(events_named(&events, "host_command_operational_retry").is_empty());
}

#[tokio::test]
async fn an_exit_79_without_the_unsettled_publish_line_is_the_commands_own_completion() {
    let fixture = fixture(vec![Scripted::Stderr(
        EXIT_UNSETTLED_PUBLISH,
        "unrelated".into(),
    )]);
    let result = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(EXIT_UNSETTLED_PUBLISH));
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert!(events(&fixture).is_empty());
}
