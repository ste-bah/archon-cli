use super::executor_process::run_command;

pub(super) fn spawn_background(
    command: String,
    input: serde_json::Value,
    cwd: std::path::PathBuf,
    session_id: String,
    event_name: String,
    timeout_secs: u32,
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
                return;
            }
        };
        if let Err(error) = run_command(
            &command,
            &payload_bytes,
            &cwd,
            &session_id,
            &event_name,
            timeout_secs,
        )
        .await
        {
            tracing::warn!(
                hook = %command,
                event = %event_name,
                error = %error,
                "background hook execution failed"
            );
        }
    });
}
