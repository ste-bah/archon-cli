#[tracing_test::traced_test]
#[test]
fn learning_decompose_best_effort_fold_calls_once_and_handles_both_results() {
    let error_calls = std::cell::Cell::new(0);
    let fold_result: Option<()> =
        super::workflow_decompose_learning::best_effort_fold("run-1", || {
            error_calls.set(error_calls.get() + 1);
            Err(anyhow::anyhow!("db offline"))
        });
    assert_eq!(error_calls.get(), 1);
    assert!(fold_result.is_none());
    assert!(logs_contain("run_id=run-1"));
    assert!(logs_contain("error=db offline"));

    let success_calls = std::cell::Cell::new(0);
    let fold_result = super::workflow_decompose_learning::best_effort_fold("run-2", || {
        success_calls.set(success_calls.get() + 1);
        Ok("folded")
    });
    assert_eq!(success_calls.get(), 1);
    assert_eq!(fold_result, Some("folded"));
}
