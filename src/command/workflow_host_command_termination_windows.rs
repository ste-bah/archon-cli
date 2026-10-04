//! Windows termination (Issue 273): the host command and everything it
//! starts live in one Job Object this supervisor owns. Every exit path,
//! a dropped supervisor included, terminates the job and waits, within a
//! bound, until no process in it is active. A job still active then is a
//! stall, which the supervisor reports as an operational, resumable outcome.
use std::sync::Arc;

use archon_shell::job_object::Job;
use archon_workflow::{WorkflowError, WorkflowResult};

use super::super::REAP_DEADLINE;
use super::{Teardown, Tree, reap};

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

impl Tree {
    /// The job holds every descendant already; there is nothing to scan.
    pub(in super::super) async fn refresh(&self) {}

    /// A job does not depend on the leader's pid.
    pub(in super::super) fn leader_reaped(&self) {}
}

/// Terminate the job and wait until it is empty, within [`REAP_DEADLINE`].
fn kill_job(job: &Job) -> Teardown {
    match job.kill_and_confirm(REAP_DEADLINE) {
        Ok(0) => Teardown::Confirmed,
        Ok(active) => Teardown::stalled(format!(
            "host command job object still has {active} active process(es) after termination"
        )),
        Err(error) => {
            Teardown::stalled(format!("terminating the host command job failed: {error}"))
        }
    }
}

async fn kill_job_off_thread(tree: &Tree) -> Teardown {
    let Some(job) = tree.job.clone() else {
        return Teardown::Confirmed;
    };
    tokio::task::spawn_blocking(move || kill_job(&job))
        .await
        .unwrap_or_else(|error| Teardown::stalled(format!("job termination task failed: {error}")))
}

pub(in super::super) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    tree: &Tree,
) -> Teardown {
    if tree.job.is_none() {
        let _ = child.start_kill();
    }
    let teardown = kill_job_off_thread(tree).await;
    match reap(child).await {
        Ok(()) => teardown,
        Err(evidence) => teardown.and_stalled(evidence),
    }
}

pub(in super::super) async fn terminate_completed_group(tree: &Tree) -> Teardown {
    kill_job_off_thread(tree).await
}

/// The drop guard's teardown: synchronous, because a drop cannot await, and
/// still confirmed, within the same bound, before the guard lets go.
pub(in super::super) fn kill_blocking(tree: &Tree, _leader_unreaped: bool) -> Teardown {
    tree.job.as_deref().map_or(Teardown::Confirmed, kill_job)
}
