//! The wave verify command, run under the same repo lock as the apply.
//!
//! Split from `patch_apply.rs` to hold the 500-line ceiling.

use std::path::Path;
use std::time::SystemTime;

use super::persist::{persist_verify, utf8_safe_tail};
use super::{ApplyError, VerifyResult, WaveId};

const TAIL_BYTES: usize = 4096;

/// MUST be invoked from inside the SAME `with_repo_lock` closure that called
/// apply_wave. The caller sequences both inside ONE closure so the lock is
/// contiguously held.
pub fn run_wave_verify(
    canonical_root: &Path,
    verify_command: Option<&str>,
    wave_id: WaveId,
    run_root: &Path,
    stage_id: &str,
) -> Result<VerifyResult, ApplyError> {
    let Some(cmd) = verify_command else {
        let result = VerifyResult {
            exit: 0,
            command: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            duration_ms: 0,
        };
        persist_verify(run_root, stage_id, wave_id, &result)?;
        return Ok(result);
    };
    let start = SystemTime::now();
    let output = std::process::Command::new(crate::acceptance::shell_program())
        .arg("-c")
        .arg(cmd)
        .current_dir(canonical_root)
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ApplyError::GitMissing
            } else {
                ApplyError::LockIo(source)
            }
        })?;
    let duration_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);
    let result = VerifyResult {
        exit: output.status.code().unwrap_or(-1),
        command: Some(cmd.to_string()),
        stdout_tail: utf8_safe_tail(&output.stdout, TAIL_BYTES),
        stderr_tail: utf8_safe_tail(&output.stderr, TAIL_BYTES),
        duration_ms,
    };
    persist_verify(run_root, stage_id, wave_id, &result)?;
    if result.exit != 0 {
        return Err(ApplyError::VerifyFailed {
            exit: result.exit,
            stderr_tail: result.stderr_tail.clone(),
        });
    }
    Ok(result)
}
