//! A declared contract verifier's output, read within a bound (Issue 219).
//!
//! A generated verifier prints one failure per missing instance, so its
//! stdout grows with the defect. Each stream is read through the scratch
//! runner's counted drain: at most [`VERIFIER_OUTPUT_BYTES`] are kept and the
//! total is counted, so the caller can mark the cut with its byte count and
//! never read a verdict it could not see whole as a pass.

use std::sync::atomic::AtomicBool;

/// Most bytes of each stream a verifier's verdict is read from.
pub(in crate::v2::verification) const VERIFIER_OUTPUT_BYTES: usize = 1024 * 1024;

/// What the verifier printed, within the bound, and how it exited.
pub(in crate::v2::verification) struct VerifierOutput {
    pub(in crate::v2::verification) status: std::process::ExitStatus,
    pub(in crate::v2::verification) stdout: Vec<u8>,
    /// Bytes the verifier wrote to stdout in all.
    pub(in crate::v2::verification) stdout_total: u64,
    pub(in crate::v2::verification) stderr: Vec<u8>,
}

/// Wait for `child`, reading both streams within the bound.
pub(in crate::v2::verification) async fn bounded_output(
    mut child: tokio::process::Child,
) -> std::io::Result<VerifierOutput> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (out_flag, err_flag) = (AtomicBool::new(false), AtomicBool::new(false));
    let read_out = async {
        match stdout {
            Some(pipe) => {
                crate::acceptance_scratch::drain_counted(pipe, VERIFIER_OUTPUT_BYTES, &out_flag)
                    .await
            }
            None => Ok((Vec::new(), 0)),
        }
    };
    let read_err = async {
        match stderr {
            Some(pipe) => {
                crate::acceptance_scratch::drain_counted(pipe, VERIFIER_OUTPUT_BYTES, &err_flag)
                    .await
            }
            None => Ok((Vec::new(), 0)),
        }
    };
    let (out, err, status) = tokio::join!(read_out, read_err, child.wait());
    let (stdout, stdout_total) = out?;
    Ok(VerifierOutput {
        status: status?,
        stdout,
        stdout_total,
        stderr: err?.0,
    })
}
