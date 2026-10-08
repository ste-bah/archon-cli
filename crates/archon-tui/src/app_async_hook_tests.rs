use super::App;

#[test]
fn async_hook_failure_diagnostic_does_not_end_the_turn() {
    let mut app = App::new();
    app.on_generation_started();
    assert!(app.is_generating);
    app.on_diagnostic_line("async hook PostToolUse [failure] source=project — exit 1");
    assert!(
        app.is_generating,
        "diagnostics are observational, not turn control"
    );
}
