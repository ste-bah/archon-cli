//! Windows termination (Issue 273): the host command and everything it
//! starts live in one Job Object this supervisor owns. Every exit path
//! terminates the job and confirms that no process in it is still active;
//! a dropped supervisor closes the job, which kills what is left.
use std::sync::Arc;
use std::time::Duration;

use archon_shell::job_object::Job;
use archon_workflow::{WorkflowError, WorkflowResult};

use super::super::REAP_DEADLINE;
use super::{Tree, reap};

const AUDIT_ATTEMPTS: u32 = 25;
const AUDIT_INTERVAL: Duration = Duration::from_millis(40);

fn failed(what: &str, error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::StageFailed(format!("{what}: {error}"))
}

/// Puts a child spawned with `CREATE_SUSPENDED_FLAG` into a fresh, named
/// job and lets it run. The name lets a resume ask whether the job still
/// runs after this executor died. A child that cannot be confined is
/// killed: it has run no code yet.
pub(in super::super) fn confine(child: &mut tokio::process::Child) -> WorkflowResult<Tree> {
    let (Some(pid), Some(handle)) = (child.id(), child.raw_handle()) else {
        let _ = child.start_kill();
        return Err(WorkflowError::StageFailed(
            "host command has no process handle to confine".to_string(),
        ));
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let name = format!(
        "Local\\archon-host-command-{}-{pid}-{stamp}",
        std::process::id()
    );
    let job = match Job::create(Some(&name)) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.start_kill();
            return Err(failed("creating the host command job object failed", error));
        }
    };
    job.adopt_suspended(handle, pid)
        .map_err(|error| failed("confining the host command in its job object failed", error))?;
    Ok(Tree {
        leader: Some(pid),
        job: Some(Arc::new(job)),
    })
}

/// Terminates the job and waits until it is empty, off the runtime thread.
/// Returns how many processes were still active when the wait ran out.
async fn kill_job(tree: &Tree) -> WorkflowResult<u32> {
    let Some(job) = tree.job.clone() else {
        return Ok(0);
    };
    tokio::task::spawn_blocking(move || job.kill_and_confirm(REAP_DEADLINE))
        .await
        .map_err(|error| failed("host command job termination task failed", error))?
        .map_err(|error| failed("terminating the host command job failed", error))
}

fn still_active(active: u32) -> WorkflowError {
    WorkflowError::StageFailed(format!(
        "host command job object still has {active} active process(es) after termination"
    ))
}

pub(in super::super) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    tree: &Tree,
) -> WorkflowResult<Vec<u32>> {
    if tree.job.is_none() {
        let _ = child.start_kill();
    }
    // A job still active here is left for the audit, so that a control
    // interruption can still report itself.
    kill_job(tree).await?;
    reap(child).await?;
    Ok(Vec::new())
}

pub(in super::super) async fn terminate_completed_group(tree: &Tree) -> WorkflowResult<()> {
    match kill_job(tree).await? {
        0 => Ok(()),
        active => Err(still_active(active)),
    }
}

pub(in super::super) async fn audit_no_descendants(
    tree: &Tree,
    _killed: Vec<u32>,
) -> WorkflowResult<()> {
    let Some(job) = &tree.job else {
        return Ok(());
    };
    let mut active = 0;
    for attempt in 0..AUDIT_ATTEMPTS {
        active = job
            .active_processes()
            .map_err(|error| failed("auditing the host command job failed", error))?;
        if active == 0 {
            return Ok(());
        }
        if attempt + 1 < AUDIT_ATTEMPTS {
            tokio::time::sleep(AUDIT_INTERVAL).await;
        }
    }
    Err(still_active(active))
}

/// Terminates the job at once; the job handle the tree holds closes when the
/// guard drops it, which kills anything that started in between.
pub(in super::super) fn kill_on_drop(tree: &Tree, _leader_unreaped: bool) {
    if let Some(job) = &tree.job {
        let _ = job.terminate();
    }
}
