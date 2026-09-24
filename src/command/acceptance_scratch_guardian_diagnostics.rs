//! What the parent hands the observation child, and what it can say when that
//! child dies. Both halves exist so an operator is never left with an exit
//! status and a path to a directory nothing wrote.
use archon_workflow::acceptance_scratch::ScratchPolicy;
use std::ffi::OsString;
use std::path::Path;

/// Cap on the child stderr carried into a stage error. The child's own error
/// line is a few hundred bytes; the cap is what keeps a runaway stream out of
/// the run record.
const DIAGNOSTIC_BYTES: usize = 4096;
/// How long the parent waits for the stderr reader once the child is gone.
const DRAIN_GRACE_SECS: u64 = 5;

/// The child's entire environment. `PATH` comes from the policy's configured
/// toolchain rather than a literal, so a check can find the binaries the host
/// declared; every other host binding is dropped unless the policy names it in
/// its allowlist. The caller clears the inherited environment first, so this is
/// the whole of what crosses the boundary.
pub(crate) fn child_environment(
    policy: &ScratchPolicy,
    host: impl Fn(&str) -> Option<OsString>,
) -> Vec<(String, OsString)> {
    let mut bindings = vec![("PATH".to_string(), policy.toolchain_path.clone().into())];
    for key in &policy.environment_allowlist {
        // `validate` already refuses an allowlist that names PATH or any other
        // host execution binding, so a named key can never displace the
        // toolchain above.
        if let Some(value) = host(key) {
            bindings.push((key.clone(), value));
        }
    }
    bindings
}

/// Read the child's stderr to end of pipe, keeping a bounded prefix.
pub(crate) async fn collect_diagnostics(mut stderr: tokio::process::ChildStderr) -> String {
    use tokio::io::AsyncReadExt;
    let mut kept: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let taken = DIAGNOSTIC_BYTES.saturating_sub(kept.len()).min(read);
                kept.extend_from_slice(&buffer[..taken]);
                truncated |= taken < read;
            }
        }
    }
    let mut text = String::from_utf8_lossy(&kept).trim().to_string();
    if truncated {
        text.push_str(" [stderr truncated]");
    }
    text
}

/// Join the reader started at spawn. A child that never exited cannot close
/// its pipe, so the wait is bounded and an unjoinable reader yields nothing
/// rather than holding the stage error hostage.
pub(crate) async fn drain(reader: Option<tokio::task::JoinHandle<String>>) -> String {
    let Some(mut reader) = reader else {
        return String::new();
    };
    let grace = std::time::Duration::from_secs(DRAIN_GRACE_SECS);
    match tokio::time::timeout(grace, &mut reader).await {
        Ok(joined) => joined.unwrap_or_default(),
        Err(_) => {
            reader.abort();
            String::new()
        }
    }
}

/// The diagnosable half of a guardian failure: what the child said, and
/// whether the evidence directory it was asked to write exists at all. Naming
/// a directory that was never created sends an operator to an empty path and
/// hides that the failure happened before any evidence could be produced.
pub(crate) fn failure_context(evidence: &Path, diagnostics: &str) -> String {
    let located = if evidence.is_dir() {
        format!("evidence: {}", evidence.display())
    } else {
        format!(
            "no evidence directory was written at {}",
            evidence.display()
        )
    };
    if diagnostics.is_empty() {
        format!("{located}; the guardian wrote nothing to stderr")
    } else {
        format!("{located}; guardian stderr: {diagnostics}")
    }
}
