use super::*;

#[tokio::test]
async fn completed_phases_are_kept_and_the_open_body_resumes_from_its_findings() {
    let run = Run::new();
    run.authors.strong.store(true, Ordering::SeqCst);
    run.authors.weak_body.store(true, Ordering::SeqCst);
    let error = run
        .run(&earlier_script(), run.args())
        .await
        .expect_err("the weak body stalls");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(run.authors.tasks().len(), 3 + 1 + 4);
    run.authors.weak_body.store(false, Ordering::SeqCst);
    let (acceptance, skeleton) = (
        run.judge.runs("freeze-acceptance"),
        run.judge.runs("freeze-skeleton"),
    );
    let tasks = run.authors.tasks().len();
    let args = run.upgrade(&[("new-script", "next-rev")]);
    let summary = run
        .run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect("the seeded run completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    let new_tasks = run.authors.tasks()[tasks..].to_vec();
    assert_eq!(new_tasks.len(), 1, "only the open body: {new_tasks:?}");
    assert!(
        new_tasks[0].contains("body is weak") && new_tasks[0].contains("Preserve every frozen"),
        "{}",
        new_tasks[0]
    );
    assert_eq!(
        run.judge.runs("freeze-acceptance"),
        acceptance,
        "the committed acceptance freeze answers from its record"
    );
    assert_eq!(
        run.judge.runs("freeze-skeleton"),
        skeleton,
        "so does the committed skeleton freeze"
    );
    assert!(run.calls().contains("body-TASK-1-author-5"));
}
