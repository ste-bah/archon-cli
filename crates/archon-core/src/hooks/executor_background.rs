use super::super::types::{HookConfig, HookOutcome};
use super::executor_process::run_command;
use super::{HookExecutionResult, RunError, hook_failure_execution_result, interpret_exit_code};

pub(super) fn spawn_background(
    command: String,
    input: serde_json::Value,
    cwd: std::path::PathBuf,
    session_id: String,
    event_name: String,
    config: HookConfig,
    source: Option<String>,
    complete: impl FnOnce(super::super::async_diagnostics::AsyncHookDiagnostic) + Send + 'static,
) {
    tokio::spawn(async move {
        let payload_bytes = match serde_json::to_vec(&input) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(
                    hook = %command,
                    event = %event_name,
                    error = %error,
                    "failed to serialize background hook payload"
                );
                complete(super::super::async_diagnostics::AsyncHookDiagnostic::new(
                    &event_name,
                    source,
                    "failure",
                    format!("could not serialize hook input: {error}"),
                ));
                return;
            }
        };
        let result = run_command(
            &command,
            &payload_bytes,
            &cwd,
            &session_id,
            &event_name,
            config.timeout.unwrap_or(60),
        )
        .await;
        let (execution, error_outcome): (HookExecutionResult, Option<&str>) = match result {
            Ok(output) => (interpret_exit_code(&config, output).into(), None),
            Err(error) => {
                let outcome = classify_run_error(&error);
                (
                    hook_failure_execution_result(&config, &event_name, &error),
                    Some(outcome),
                )
            }
        };
        let no_progress = execution.no_progress_stop.is_some();
        let result = execution.result;
        let outcome = if no_progress {
            "no_progress_stop"
        } else if let Some(error_outcome) = error_outcome {
            error_outcome
        } else {
            match result.outcome {
                HookOutcome::Success => "success",
                HookOutcome::NonBlockingError => "failure",
                HookOutcome::Blocking => "failure",
                HookOutcome::Cancelled => "failure",
            }
        };
        let message = result.reason.unwrap_or_else(|| match outcome {
            "success" => "completed".to_owned(),
            _ => format!("{:?}", result.outcome),
        });
        complete(super::super::async_diagnostics::AsyncHookDiagnostic::new(
            &event_name,
            source,
            outcome,
            message,
        ));
    });
}

fn classify_run_error(error: &RunError) -> &'static str {
    if error.is_no_progress() {
        "no_progress_stop"
    } else if matches!(error, RunError::Timeout(_)) {
        "timeout"
    } else {
        "failure"
    }
}

#[cfg(test)]
mod tests {
    use super::classify_run_error;
    use crate::hooks::executor::RunError;

    #[test]
    fn spawn_deadline_is_reported_as_timeout() {
        assert_eq!(
            classify_run_error(&RunError::Timeout("process spawn")),
            "timeout"
        );
    }

    #[test]
    fn stalled_work_is_reported_as_no_progress_stop() {
        assert_eq!(
            classify_run_error(&RunError::Timeout("pipe drain")),
            "no_progress_stop"
        );
    }

    #[test]
    fn launch_errors_are_reported_as_failures() {
        assert_eq!(
            classify_run_error(&RunError::Spawn("blocked".into())),
            "failure"
        );
    }
}
