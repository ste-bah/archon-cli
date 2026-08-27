use super::*;

use super::{finish_loop_result, slash_command_name, slash_input};

#[test]
fn signal_registration_and_shutdown_failures_remain_visible() {
    let result = finish_loop_result(
        Err(anyhow::anyhow!("audit shutdown failed")),
        Some(anyhow::anyhow!("SIGTERM registration failed")),
    )
    .expect_err("signal and shutdown failures must remain visible");
    let message = result.to_string();

    assert!(
        message.contains("SIGTERM registration failed"),
        "{result:#}"
    );
    assert!(message.contains("audit shutdown failed"), "{result:#}");
}

#[test]
fn decomposition_and_session_shutdown_failures_remain_visible() {
    let error = combine_shutdown_results(
        Err(anyhow::anyhow!("decomposition reap failed")),
        Err(anyhow::anyhow!("audit drain failed")),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("decomposition reap failed"), "{error}");
    assert!(error.contains("audit drain failed"), "{error}");
}

#[test]
fn finish_loop_reports_loop_and_shutdown_failures_together() {
    let result = finish_loop_result(
        Err(anyhow::anyhow!("audit shutdown failed")),
        Some(anyhow::anyhow!("loop failed")),
    )
    .expect_err("both session-loop failures must remain visible");
    let message = result.to_string();

    assert!(message.contains("loop failed"), "{result:#}");
    assert!(message.contains("audit shutdown failed"), "{result:#}");
}

#[test]
fn slash_input_allows_leading_whitespace() {
    assert_eq!(
        slash_input("  /cognitive daemon start"),
        Some(std::borrow::Cow::Borrowed("/cognitive daemon start"))
    );
}

#[test]
fn slash_input_rejects_plain_prompt() {
    assert_eq!(slash_input("hello /cognitive"), None);
}

#[test]
fn slash_command_name_returns_first_token() {
    assert_eq!(slash_command_name("/cognitive daemon start"), "/cognitive");
}

#[test]
fn slash_input_accepts_copied_tui_prompt_marker() {
    assert_eq!(
        slash_input("> /workflow run --live build it"),
        Some(std::borrow::Cow::Borrowed("/workflow run --live build it"))
    );
}

#[test]
fn slash_input_normalizes_tui_cli_workflow_command() {
    assert_eq!(
        slash_input("./archon workflow resume --live wf-123"),
        Some(std::borrow::Cow::Owned(
            "/workflow resume --live wf-123".to_string()
        ))
    );
}

#[test]
fn slash_input_normalizes_absolute_cli_workflow_command() {
    assert_eq!(
        slash_input("/tmp/project/archon workflow run --live do work"),
        Some(std::borrow::Cow::Owned(
            "/workflow run --live do work".to_string()
        ))
    );
}

#[test]
fn slash_input_preserves_decomposed_workflow_flag() {
    assert_eq!(
        slash_input("./archon workflow run --live --decomposed do work"),
        Some(std::borrow::Cow::Owned(
            "/workflow run --live --decomposed do work".to_string()
        ))
    );
}
