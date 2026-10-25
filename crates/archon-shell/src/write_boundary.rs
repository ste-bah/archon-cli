//! Issue-227: which operating-system write boundary this process can put
//! under a child it starts, asked once, for every caller that bounds one
//! (the Bash tool's shell, the host-run verifiers of `archon-workflow`).
//!
//! - macOS: `sandbox-exec` with a profile (`bash_write_sandbox`), applied by
//!   a wrapper that execs the command in place.
//! - Linux: Landlock ([`landlock`]), applied to the child between `fork` and
//!   `exec`, so nothing the command runs can lift it.
//! - Anything else (Windows, and any platform with neither `sandbox-exec` nor
//!   Landlock): a host-side snapshot-and-restore boundary
//!   ([`Mechanism::HostSnapshot`], `write_boundary_snapshot`). It is not a
//!   kernel confinement — it records the sealed roots, runs the command, then
//!   restores and names any change — so it fails the call after the fact rather
//!   than refusing the write, but nothing a command writes under a sealed root
//!   persists and no verdict taken from a tampered tree stands (Issue-234).
//!
//! A caller that requires a bounded child (a Bash shell or host-run verifier)
//! refuses to run it ([`refusal`]). A caller must also
//! refuse when the host did not stamp the boundary paths: without that stamp
//! it cannot identify which roots to seal.

use std::sync::OnceLock;

#[path = "write_boundary_landlock.rs"]
pub mod landlock;

#[path = "write_boundary_snapshot.rs"]
pub mod snapshot;
pub use snapshot::{SnapshotBoundary, SnapshotViolation};

/// The macOS sandbox wrapper.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The mechanism that bounds a child's writes here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// `sandbox-exec` with a computed profile.
    SandboxExec,
    /// A Landlock ruleset of this ABI version.
    Landlock { abi: u32 },
    /// A host-side snapshot-and-restore boundary (`write_boundary_snapshot`):
    /// the fallback where no kernel confinement exists (Windows, Issue-234).
    HostSnapshot,
}

/// The boundary this process can apply, or why there is none; probed once.
pub fn mechanism() -> Result<Mechanism, String> {
    static PROBED: OnceLock<Result<Mechanism, String>> = OnceLock::new();
    PROBED
        .get_or_init(|| {
            let probed = probe();
            match &probed {
                Ok(mechanism) => tracing::info!(?mechanism, "OS write boundary available"),
                Err(reason) => tracing::warn!(reason, "no OS write boundary in this process"),
            }
            probed
        })
        .clone()
}

/// The text a caller that requires a bounded child gives instead of running
/// it unbounded: names the platform, the reason and what was refused. Logged
/// as an error here, so no caller can refuse silently.
pub fn refusal(what: &str, reason: &str) -> String {
    let text = format!(
        "{what} requires an OS write boundary and none can be applied on this host ({os}): \
         {reason}. It was NOT run: running it unbounded would let it write the project, \
         repository and run roots the boundary exists to seal.",
        os = std::env::consts::OS
    );
    tracing::error!("{text}");
    text
}

/// The host failed to stamp the roots this boundary must seal.
pub fn missing_stamp_refusal(what: &str) -> String {
    let text = format!(
        "{what} was refused: the host did not provide the boundary paths (stamp, worktree root \
         or protected roots), so it cannot seal the project, repository and run roots. Report \
         this in your result."
    );
    tracing::error!("{text}");
    text
}

#[cfg(target_os = "macos")]
fn probe() -> Result<Mechanism, String> {
    let status = crate::spawn::command(SANDBOX_EXEC)
        .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => Ok(Mechanism::SandboxExec),
        Ok(status) => Err(format!(
            "{SANDBOX_EXEC} cannot apply a profile in this process ({status}); an \
             already-sandboxed process cannot start another sandbox"
        )),
        Err(error) => Err(format!("{SANDBOX_EXEC} cannot be started: {error}")),
    }
}

#[cfg(target_os = "linux")]
fn probe() -> Result<Mechanism, String> {
    landlock::probe().map(|abi| Mechanism::Landlock { abi })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn probe() -> Result<Mechanism, String> {
    // Issue-234: no kernel confinement here, so the host-side snapshot boundary
    // is the guarantee. It can always be taken (it only reads and writes the
    // filesystem), so this host never refuses a bounded child for want of a
    // mechanism.
    Ok(Mechanism::HostSnapshot)
}
