//! Shared job teardown orchestration; Windows supplies kernel operations.
use super::Teardown;
use crate::command::workflow_host_command_groups::GroupEvidence;
use archon_shell::teardown_progress::Progress;
use std::{io, sync::Arc, time::Duration};

pub(super) trait JobOps: Send + Sync + 'static {
    fn process_identities_observed(&self, progress: &Progress) -> io::Result<Vec<(u32, u64)>>;
    fn kill_and_confirm_observed(&self, progress: &Progress) -> io::Result<u32>;
}
#[cfg(windows)]
impl JobOps for archon_shell::job_object::Job {
    fn process_identities_observed(&self, progress: &Progress) -> io::Result<Vec<(u32, u64)>> {
        self.process_identities_observed(progress)
    }
    fn kill_and_confirm_observed(&self, progress: &Progress) -> io::Result<u32> {
        self.kill_and_confirm_observed(progress)
    }
}

/// Each phase observes blocking admission, locks and I/O. Only completed
/// identity reads, durable checkpoints or decreasing accounting are progress.
async fn phase<T: Send + 'static>(
    bound: Duration,
    work: impl FnOnce(Progress) -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let progress = Progress::new(bound);
    let observed = progress.clone();
    progress
        .watch(tokio::task::spawn_blocking(move || work(observed)))
        .await?
}

pub(super) async fn kill_job_off_thread(
    job: Arc<impl JobOps>,
    evidence: Option<GroupEvidence>,
    bound: Duration,
) -> Teardown {
    let preparing_job = job.clone();
    let preparing_evidence = evidence.clone();
    // Registration is already durably incomplete. No synchronous checkpoint
    // is needed before the watched worker has been admitted.
    let preparation = phase(bound, move |progress| {
        if let Some(evidence) = &preparing_evidence {
            evidence.begin_observed(&progress)?;
        }
        let pins = preparing_job.process_identities_observed(&progress)?;
        if let Some(evidence) = &preparing_evidence {
            evidence.remember(&pins)?;
        }
        progress.check()?;
        Ok(())
    })
    .await;
    let confirming_job = job.clone();
    // Even failed persistence must still attempt termination.
    let killed = phase(bound, move |progress| {
        confirming_job.kill_and_confirm_observed(&progress)
    })
    .await;
    // Expired accounting says nothing about how long collecting identities
    // will take. Collection has a fresh clock AND its own async watchdog.
    let collecting_evidence = evidence.clone();
    let confirmed = matches!(killed, Ok(0));
    let collected = phase(bound, move |progress| {
        let pins = if confirmed { Vec::new() } else { job.process_identities_observed(&progress)? };
        // A nonzero accounting result with no live identities is inconclusive.
        if !confirmed && pins.is_empty() {
            return Err(io::Error::other("job accounting has not confirmed exit and no survivor identities could be collected"));
        }
        if preparation.is_ok() {
            if let Some(evidence) = &collecting_evidence { evidence.complete(&pins)?; }
        }
        progress.check()?;
        Ok((pins, preparation))
    }).await;
    let (pins, preparation) = match collected {
        Ok(collected) => collected,
        Err(error) => {
            return Teardown::stalled(format!(
                "job survivor collection stalled; survivors unknown: {error}"
            ));
        }
    };
    if let Err(error) = preparation {
        return Teardown::stalled(format!(
            "recording job identities failed; survivors unknown: {error}"
        ));
    }
    match killed {
        Ok(0) => Teardown::Confirmed,
        result => Teardown::Stalled {
            evidence: match result {
                Ok(active) => format!(
                    "host command job object still has {active} active process(es) after termination"
                ),
                Err(error) => format!("terminating the host command job stalled: {error}"),
            },
            survivors: Some(pins),
        },
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_host_command_job_r3_tests.rs"]
mod r3_tests;
