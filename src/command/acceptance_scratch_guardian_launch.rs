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
    archon_shell::spawn::replace_environment(
        command.as_std_mut(),
        child_environment(&request.policy, |key: &str| std::env::var_os(key)),
    );
    command
        // Its stderr renews the window below: it reports activity there.
        .env(archon_shell::progress::SUPERVISED_ENV, "1")
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
    // A silent check saves its result at its own window; allow teardown and
    // evidence publication before declaring the enclosing guardian stalled.
    // Actual check output, tree activity and finished phases cross stderr
    // as coalesced pulses.
    let budget = request.policy.timeout_secs.min(86400).saturating_add(120);
    #[cfg(test)]
    let budget = TEST_WINDOW.with(|window| window.get()).unwrap_or(budget);
    // #356: delivery is bounded by no progress, never a total: every chunk
    // the child takes renews the window; a child that takes none pauses.
    let delivered = progress
        .bound(std::time::Duration::from_secs(budget), async {
            for chunk in bytes.chunks(8192) {
                pipe.write_all(chunk).await?;
                progress.record();
            }
            pipe.flush().await
        })
        .await;
    match delivered {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(WorkflowError::SpecInvalid(error.to_string())),
        Err(_) => {
            let _ = child.kill().await;
            return Err(WorkflowError::ControlPaused(format!(
                "native guardian took no request byte for {budget}s; resumable; {}",
                failure_context(&request.evidence, &drain(diagnostics).await)
            )));
        }
    }
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
        // A publish no read could settle (Issue 338), or a stall (#356).
        return Err(WorkflowError::ControlPaused(format!(
            "the native observation guardian paused: {evidence}"
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
