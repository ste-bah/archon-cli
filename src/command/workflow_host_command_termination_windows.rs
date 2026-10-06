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
        evidence: None,
        job: Some(Arc::new(job)),
    })
}

impl Tree {
    /// The job holds every descendant already; there is nothing to scan.
    pub(in super::super) fn spawn_refresh(&self) -> Option<tokio::task::JoinHandle<()>> {
        None
    }

    /// A job does not depend on the leader's pid.
    pub(in super::super) fn leader_reaped(&self) {}
}

/// Resolves when the leader exits. A job does not depend on the leader's
/// pid, so waiting (which reaps it) is safe here.
pub(in super::super) async fn leader_exit(
    child: &mut tokio::process::Child,
) -> std::io::Result<()> {
    child.wait().await.map(drop)
}

/// Terminate the job and wait until it is empty, within [`REAP_DEADLINE`].
fn kill_job(
    job: &Job,
    evidence: Option<&super::super::super::workflow_host_command_groups::GroupEvidence>,
) -> Teardown {
    let persistence = evidence.and_then(|evidence| evidence.begin().err());
    let before = job.process_identities(REAP_DEADLINE);
    let recorded = match (&before, evidence) {
        (Ok(pins), Some(evidence)) => evidence.remember(pins),
        _ => Ok(()),
    };
    let killed = job.kill_and_confirm(REAP_DEADLINE);
    let survivors = job.process_identities(REAP_DEADLINE);
    let successful = killed.is_ok();
    let teardown = match killed {
        Ok(0) => Teardown::Confirmed,
        Ok(active) => Teardown::Stalled {
            evidence: format!(
                "host command job object still has {active} active process(es) after termination"
            ),
            survivors: survivors
                .as_ref()
                .ok()
                .filter(|pins| !pins.is_empty())
                .cloned(),
        },
        Err(error) => {
            Teardown::stalled(format!("terminating the host command job failed: {error}"))
        }
    };
    if let Some(error) = persistence.or_else(|| recorded.err()) {
        return teardown.and_stalled(format!("recording job identities failed: {error}"));
    }
    if successful
        && let Some(evidence) = evidence
        && let Ok(pins) = &survivors
    {
        if let Err(error) = evidence.complete(pins) {
            return teardown.and_stalled(format!("recording job identities failed: {error}"));
        }
    }
    teardown
}

async fn kill_job_off_thread(tree: &Tree) -> Teardown {
    let Some(job) = tree.job.clone() else {
        return Teardown::Confirmed;
    };
    let evidence = tree.evidence.clone();
    let work = tokio::task::spawn_blocking(move || kill_job(&job, evidence.as_ref()));
    // Enumeration before and after termination each has its own bound. Include
    // blocking-pool admission in the async bound, so a queued task also stalls.
    match tokio::time::timeout(REAP_DEADLINE * 3, work).await {
        Ok(Ok(teardown)) => teardown,
        Ok(Err(error)) => Teardown::stalled(format!("job termination task failed: {error}")),
        Err(_) => Teardown::stalled("job teardown deadline exceeded; survivors unknown"),
    }
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
        Ok(_) => teardown,
        Err(evidence) => teardown.and_stalled(evidence),
    }
}

pub(in super::super) async fn terminate_completed_group(tree: &Tree) -> Teardown {
    kill_job_off_thread(tree).await
}

/// The teardown of a supervisor that stopped without settling. Synchronous
/// and multi-second at worst: the guard runs it on a dedicated thread, never
/// on the async runtime, and keeps the resume record until it reports.
pub(in super::super) fn kill_blocking(tree: &Tree, _leader_unreaped: bool) -> Teardown {
    tree.job.as_deref().map_or(Teardown::Confirmed, |job| {
        kill_job(job, tree.evidence.as_ref())
    })
}
