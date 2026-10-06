//! Parent supervision of the owned native observer.
use super::diagnostics::{child_environment, collect_diagnostics, drain, failure_context};
use super::*;

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_WINDOW: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

pub(crate) async fn launch(request: Request) -> WorkflowResult<ObservationResult> {
    launch_selected(request, None).await
}
pub(crate) async fn launch_selected(
    request: Request,
    selection: CheckSelection,
) -> WorkflowResult<ObservationResult> {
    use tokio::io::AsyncWriteExt;
    // The toolchain the child is given below is only as trustworthy as the
    // policy carrying it.
    request.policy.validate()?;
    validate_selected(&request, &selection)?;
    let mut command = archon_shell::spawn::tokio_command(
        std::env::current_exe().map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?,
    );
    #[cfg(not(test))]
    command.arg(FLAG);
    #[cfg(test)]
    command.args([
        "--exact",
        "command::acceptance_scratch_guardian::tests::guardian_entry",
        "--ignored",
        "--nocapture",
    ]);
    command
        .env_clear()
        .envs(child_environment(&request.policy, |key: &str| {
            std::env::var_os(key)
        }))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        // Piped, never discarded: a guardian that dies before writing evidence
        // leaves its stderr as the only account of why.
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    // Drained concurrently with the wait below so a talkative child cannot
    // block on a full pipe.
    let progress = archon_shell::progress::Progress::default();
    let diagnostics = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(collect_diagnostics(stderr, progress.clone())));
    let mut pipe = child.stdin.take().unwrap();
    let bytes = request_line(&request, &selection)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), pipe.write_all(&bytes))
        .await
        .map_err(|_| WorkflowError::StageFailed("guardian request delivery timed out".into()))?
        .map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?;
    // A silent check saves its result at its own window; allow teardown and
    // evidence publication before declaring the enclosing guardian stalled.
    // Actual check output and tree activity cross stderr as coalesced pulses.
    let budget = request.policy.timeout_secs.min(86400).saturating_add(120);
    #[cfg(test)]
    let budget = TEST_WINDOW.with(|window| window.get()).unwrap_or(budget);
    let status = match progress
        .bound(std::time::Duration::from_secs(budget), child.wait())
        .await
    {
        Ok(status) => status.map_err(|e| WorkflowError::SpecInvalid(e.to_string()))?,
        Err(_) => {
            drop(pipe);
            let cleanup_grace = request
                .policy
                .timeout_secs
                .clamp(5, 86400)
                .saturating_add(10);
            if tokio::time::timeout(std::time::Duration::from_secs(cleanup_grace), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
            return Err(WorkflowError::ControlPaused(format!(
                "native guardian stalled: no child activity for {budget}s; observation evidence retained; {}",
                failure_context(&request.evidence, &drain(diagnostics).await)
            )));
        }
    };
    drop(pipe);
    let diagnostics = drain(diagnostics).await;
    if let Some(evidence) =
        crate::command::workflow_host_command_operational::unsettled_publish_evidence(
            status.code(),
            diagnostics.as_bytes(),
        )
    {
        return Err(WorkflowError::ControlPaused(format!(
            "the native observation guardian read nothing: {evidence}"
        )));
    }
    if !status.success() {
        return Err(WorkflowError::StageFailed(format!(
            "native observation guardian failed ({status}); {}",
            failure_context(&request.evidence, &diagnostics)
        )));
    }
    let path = request.evidence.join("observation.json");
    serde_json::from_slice(&std::fs::read(&path).map_err(|e| WorkflowError::Io {
        path: path.clone(),
        source: e,
    })?)
    .map_err(Into::into)
}
