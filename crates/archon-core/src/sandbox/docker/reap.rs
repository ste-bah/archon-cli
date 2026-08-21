//! Containers whose owner is gone.
//!
//! Holding a container open trades away `--rm`'s guarantee, and the guarantee
//! has to be replaced rather than assumed. Three mechanisms, deliberately
//! independent, because the first two do not run in the case that matters most:
//!
//! 1. `Drop` on the pool tears down at the session boundary. Does not run under
//!    SIGKILL, `abort`, or `std::process::exit`.
//! 2. This module, run once before the first container of a process is created,
//!    removes containers left behind by an Archon that is no longer running.
//! 3. `sleep <container_max_age_secs>` is the container's PID 1 and `--rm` is
//!    set, so it stops and is removed on its own. This is the only one that
//!    needs nothing from the host, and it is what bounds the leak when Archon is
//!    killed and never started again.

use std::process::Stdio;

use tokio::process::Command as TokioCommand;

use super::pool::{OWNED_LABEL, OWNER_LABEL, PID_LABEL, owner_id};

/// Remove every Archon sandbox container whose creating process is gone.
///
/// Ownership is checked twice and reaping needs both answers: a different owner
/// id *and* a dead pid. Parallel Archon sessions on one machine are ordinary
/// here, so "not mine" alone would have two runs destroying each other's
/// containers mid-command.
pub(super) async fn reap_orphans(binary: String) {
    let listed = match list_owned(&binary).await {
        Ok(listed) => listed,
        Err(error) => {
            tracing::debug!(%error, "sandbox: could not list containers to reap");
            return;
        }
    };
    let mut system = sysinfo::System::new();
    for candidate in listed {
        if candidate.owner == owner_id() {
            continue;
        }
        if owner_is_alive(&mut system, candidate.pid) {
            continue;
        }
        tracing::info!(
            container = %candidate.name,
            owner = %candidate.owner,
            "sandbox: removing a container left behind by a dead Archon process"
        );
        let _ = TokioCommand::new(&binary)
            .args(["rm", "--force", &candidate.name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
    }
}

pub(super) struct Orphan {
    pub(super) name: String,
    pub(super) owner: String,
    pub(super) pid: Option<u32>,
}

async fn list_owned(binary: &str) -> Result<Vec<Orphan>, String> {
    let output = TokioCommand::new(binary)
        .args([
            "ps",
            "--all",
            "--filter",
            &format!("label={OWNED_LABEL}=1"),
            "--format",
            &format!(
                "{{{{.Names}}}}\t{{{{.Label \"{OWNER_LABEL}\"}}}}\t{{{{.Label \"{PID_LABEL}\"}}}}"
            ),
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| format!("failed to spawn docker: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_listing(&String::from_utf8_lossy(&output.stdout)))
}

pub(super) fn parse_listing(stdout: &str) -> Vec<Orphan> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(Orphan {
                name: name.to_string(),
                owner: fields.next().unwrap_or_default().trim().to_string(),
                pid: fields.next().and_then(|pid| pid.trim().parse().ok()),
            })
        })
        .collect()
}

/// Whether the process that created a container is still running.
///
/// An unparseable or absent pid label counts as dead: it is a container this
/// build did not create, or one whose label was lost, and either way nothing is
/// going to tear it down.
///
/// Pid reuse can make a dead owner look alive. That error is the safe one — the
/// container is kept and its own `sleep` timeout removes it — and the opposite
/// error, killing a live session's container, is the one this avoids.
pub(super) fn owner_is_alive(system: &mut sysinfo::System, pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system
        .process(pid)
        .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie)
}

#[cfg(test)]
#[path = "reap_tests.rs"]
mod tests;
