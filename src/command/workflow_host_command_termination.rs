//! Platform-specific termination.
//!
//! On Unix a host command runs as the leader of its own session (Issue 270):
//! a nested runner may give each of its checks a process group of its own,
//! and a group kill never reaches those. Teardown therefore works on the
//! whole tree - the command's group, its session, and every descendant by
//! ancestry (`archon_shell::process_tree`) - and is confirmed only when no
//! live member of any of them is left.
use archon_workflow::{WorkflowError, WorkflowResult};

use super::REAP_DEADLINE;
#[cfg(unix)]
use super::{CLEANUP_GRACE, DESCENDANT_AUDIT_ATTEMPTS, DESCENDANT_AUDIT_INTERVAL};
#[cfg(unix)]
use archon_shell::process_tree::Scope;

/// The tree of the command whose leader is `leader`. Ancestry from the
/// leader itself is used only while it is unreaped (`leader_unreaped`): once
/// reaped, its pid may already belong to an unrelated process.
#[cfg(unix)]
pub(super) fn scope(leader: u32, leader_unreaped: bool) -> Scope {
    Scope {
        roots: if leader_unreaped {
            vec![leader]
        } else {
            Vec::new()
        },
        groups: vec![leader],
        sessions: vec![leader],
    }
}

/// Runs a blocking process-table operation off the runtime thread.
#[cfg(unix)]
async fn blocking<T: Send + 'static>(
    what: &'static str,
    work: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> WorkflowResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| WorkflowError::StageFailed(format!("{what} task failed: {error}")))?
        .map_err(|error| WorkflowError::StageFailed(format!("{what} failed: {error}")))
}

#[cfg(unix)]
fn survivors_error(leader: u32, survivors: &[u32]) -> WorkflowError {
    WorkflowError::StageFailed(format!(
        "host command process tree {leader} still has live members after termination: {}",
        survivors
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Asks the tree to stop, kills what is left after the grace, and reaps the
/// direct child. Returns the members that survived the kill, for
/// [`audit_no_descendants`]: some were reachable only through the leader,
/// which is reaped now, so the audit could no longer find them itself. They
/// are returned rather than raised so that a control interruption can still
/// report itself.
#[cfg(unix)]
pub(super) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    process_group: Option<u32>,
) -> WorkflowResult<Vec<u32>> {
    let mut survivors = Vec::new();
    if let Some(leader) = process_group {
        let tree = scope(leader, true);
        let asked = tree.clone();
        blocking("signalling the host command tree", move || {
            asked.signal(libc::SIGTERM)
        })
        .await?;
        tokio::time::sleep(CLEANUP_GRACE).await;
        survivors = blocking("killing the host command tree", move || {
            tree.kill(REAP_DEADLINE)
        })
        .await?;
    } else {
        let _ = child.start_kill();
    }
    reap(child).await?;
    Ok(survivors)
}

/// After the leader exited on its own: kill whatever it left behind and
/// confirm the tree is empty.
#[cfg(unix)]
pub(super) async fn terminate_completed_group(process_group: Option<u32>) -> WorkflowResult<()> {
    let Some(leader) = process_group else {
        return Ok(());
    };
    let tree = scope(leader, false);
    let survivors = blocking("killing the host command tree", move || {
        tree.kill(REAP_DEADLINE)
    })
    .await?;
    if survivors.is_empty() {
        Ok(())
    } else {
        Err(survivors_error(leader, &survivors))
    }
}

/// Confirms no member of the tree survived termination.
///
/// Killing a tree and verifying nothing survived it are different
/// obligations. Every capability declares `detaches: false`, which makes a
/// survivor here a broken invariant rather than an expected case. The retry
/// exists because a just-killed member can still be in the process table for
/// a moment; only a member that outlives the whole window is reported.
///
/// `killed` are the members a kill left alive. A pid among them that is
/// still in the table is still a survivor: a pid is not reused within the
/// audit window on any system this runs on.
#[cfg(unix)]
pub(super) async fn audit_no_descendants(
    process_group: Option<u32>,
    killed: Vec<u32>,
) -> WorkflowResult<()> {
    let Some(leader) = process_group else {
        return Ok(());
    };
    let mut survivors = Vec::new();
    for attempt in 0..DESCENDANT_AUDIT_ATTEMPTS {
        let tree = scope(leader, false);
        let killed = killed.clone();
        survivors = blocking("auditing the host command tree", move || {
            let table = archon_shell::process_tree::snapshot()?;
            let mut members = tree.members_in(&table);
            for pid in killed {
                let alive = table.iter().any(|p| p.pid == pid && !p.zombie);
                if alive && !members.contains(&pid) {
                    members.push(pid);
                }
            }
            Ok(members)
        })
        .await?;
        if survivors.is_empty() {
            return Ok(());
        }
        if attempt + 1 < DESCENDANT_AUDIT_ATTEMPTS {
            tokio::time::sleep(DESCENDANT_AUDIT_INTERVAL).await;
        }
    }
    Err(survivors_error(leader, &survivors))
}

/// The drop guard's kill: synchronous, because a drop cannot await, and
/// short, because it runs on whatever thread dropped the supervisor.
#[cfg(unix)]
pub(super) fn kill_on_drop(leader: u32, leader_unreaped: bool) {
    let _ = scope(leader, leader_unreaped).kill(std::time::Duration::from_millis(250));
}

#[cfg(not(unix))]
pub(super) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    _process_group: Option<u32>,
) -> WorkflowResult<Vec<u32>> {
    child.start_kill().map_err(|error| {
        WorkflowError::StageFailed(format!("host command termination failed: {error}"))
    })?;
    reap(child).await?;
    Ok(Vec::new())
}

#[cfg(not(unix))]
pub(super) async fn terminate_completed_group(_process_group: Option<u32>) -> WorkflowResult<()> {
    Ok(())
}

/// Without process groups there are no descendants to audit; the child handle
/// termination is the whole cleanup.
#[cfg(not(unix))]
pub(super) async fn audit_no_descendants(
    _process_group: Option<u32>,
    _killed: Vec<u32>,
) -> WorkflowResult<()> {
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn kill_on_drop(_leader: u32, _leader_unreaped: bool) {}

async fn reap(child: &mut tokio::process::Child) -> WorkflowResult<()> {
    tokio::time::timeout(REAP_DEADLINE, child.wait())
        .await
        .map_err(|_| {
            WorkflowError::StageFailed(
                "host command process reap exceeded cleanup deadline".to_string(),
            )
        })?
        .map_err(|error| {
            WorkflowError::StageFailed(format!("host command process reap failed: {error}"))
        })?;
    Ok(())
}
