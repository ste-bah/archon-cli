//! The wave verify command, run under the same repo lock as the apply.
//!
//! Split from `patch_apply.rs` to hold the 500-line ceiling.

use std::path::Path;
use std::time::SystemTime;

use super::persist::{persist_verify, utf8_safe_tail};
use super::{ApplyError, VerifyResult, WaveId};

const TAIL_BYTES: usize = 4096;

pub(crate) fn verify_environment_ready(
    run_root: &Path,
) -> Result<crate::acceptance_check_environment::CommandEnvironment, ApplyError> {
    let policy = crate::acceptance_check_environment::policy_for_run(Some(run_root))
        .map_err(ApplyError::VerifyEnvironment)?;
    crate::acceptance_check_environment::CommandEnvironment::capture(policy.as_ref())
        .map_err(ApplyError::VerifyEnvironment)
}

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
    let environment = if verify_command.is_some_and(|command| !command.trim().is_empty()) {
        Some(verify_environment_ready(run_root)?)
    } else {
        None
    };
    run_wave_verify_prepared(
        canonical_root,
        verify_command,
        wave_id,
        run_root,
        stage_id,
        environment,
    )
}

pub(crate) fn run_wave_verify_prepared(
    canonical_root: &Path,
    verify_command: Option<&str>,
    wave_id: WaveId,
    run_root: &Path,
    stage_id: &str,
    environment: Option<crate::acceptance_check_environment::CommandEnvironment>,
) -> Result<VerifyResult, ApplyError> {
    let Some(cmd) = verify_command.filter(|command| !command.trim().is_empty()) else {
        let result = VerifyResult {
            exit: 0,
            command: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            environment_note: None,
            duration_ms: 0,
        };
        persist_verify(run_root, stage_id, wave_id, &result)?;
        return Ok(result);
    };
    let environment = environment
        .ok_or_else(|| ApplyError::VerifyEnvironment("verify environment was not prepared".into()))?
        .with_remedy(crate::acceptance_check_environment::RUN_POLICY_REMEDY);
    let start = SystemTime::now();
    let output = environment
        .command(crate::acceptance::shell_program())
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
    let environment_note = (!output.status.success())
        .then(|| environment.note(&[&output.stdout, &output.stderr]))
        .flatten();
    let stderr_tail = utf8_safe_tail(&output.stderr, TAIL_BYTES);
    let result = VerifyResult {
        exit: output.status.code().unwrap_or(-1),
        command: Some(cmd.to_string()),
        stdout_tail: utf8_safe_tail(&output.stdout, TAIL_BYTES),
        stderr_tail,
        environment_note,
        duration_ms,
    };
    persist_verify(run_root, stage_id, wave_id, &result)?;
    if result.exit != 0 {
        return Err(ApplyError::VerifyFailed {
            exit: result.exit,
            stderr_tail: result.stderr_tail.clone(),
            environment_note: result.environment_note.clone(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod environment_preflight_tests {
    use super::*;

    fn run_root(metadata: &[u8]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("v2")).unwrap();
        std::fs::write(root.path().join("v2/generated-metadata.json"), metadata).unwrap();
        root
    }

    #[test]
    fn absent_required_operator_variable_is_an_operational_preflight_error() {
        let root = run_root(
            br#"{"check_environment_policy":{"toolchain_path":"/usr/bin:/bin","bound":{},"forwarded":["ARCHON_349_REQUIRED_DATA"]}}"#,
        );
        let Err(error) = verify_environment_ready(root.path()) else {
            panic!("missing operator variable unexpectedly passed preflight");
        };
        assert!(matches!(error, ApplyError::VerifyEnvironment(_)));
    }

    #[test]
    fn unreadable_policy_metadata_is_an_operational_preflight_error() {
        let root = run_root(b"not-json");
        let Err(error) = verify_environment_ready(root.path()) else {
            panic!("malformed metadata unexpectedly passed preflight");
        };
        assert!(matches!(error, ApplyError::VerifyEnvironment(_)));
    }

    #[test]
    fn invalid_recorded_policy_is_an_operational_preflight_error() {
        let root = run_root(
            br#"{"check_environment_policy":{"toolchain_path":null,"bound":{},"forwarded":[]}}"#,
        );
        let Err(error) = verify_environment_ready(root.path()) else {
            panic!("invalid policy unexpectedly passed preflight");
        };
        assert!(matches!(error, ApplyError::VerifyEnvironment(_)));
    }
}
