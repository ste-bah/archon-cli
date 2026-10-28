//! Recovery admission holds the executor lease before settling old children.
use super::*;
use crate::command::workflow_executor_lease::ExecutionLease;

/// Live resume alone may terminate children left by an exited executor.
/// Inspection and sibling-command admission retain their refusal-only path.
pub(crate) async fn settle_before_resume(
    run_dir: &Path,
    run_id: &str,
    lease: &ExecutionLease,
    check: impl Fn() -> anyhow::Result<()>,
) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    anyhow::ensure!(
        lease.is_held_by(lease.holder().pid, &lease.holder().acquired_at),
        "recorded-child recovery requires the held executor lease"
    );
    #[cfg(unix)]
    {
        let (running, mut ended) = left_groups_read_only(run_dir)?;
        for mut record in running {
            let path = record
                .file
                .clone()
                .context("recorded child has no record path")?;
            let pending = path.with_extension("pending");
            if record.survivors_unknown || owner::owner(&record, &pending) != Owner::Exited {
                // Unknown membership and a live owner retain the existing fence.
                return require_no_running_groups_from(run_dir, run_id, vec![record], ended);
            }
            let retained: Retained = Default::default();
            record = crate::command::workflow_host_retry::retry_waiting(
                &record.command_id,
                || {
                    let mut attempt = record.clone();
                    let retained = retained.clone();
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let file = attempt
                                .file
                                .clone()
                                .context("recorded child has no record path")?;
                            if let Some(marker) =
                                guard::pending_record(&file.with_extension("pending"))?
                            {
                                attempt = marker.record;
                                attempt.file = Some(file);
                            }
                            terminate_recorded(&mut attempt, &retained)?;
                            Ok(attempt)
                        })
                        .await?
                    }
                },
                |delay| {
                    crate::command::workflow_host_retry::controlled_by(&check, async move {
                        tokio::time::sleep(delay).await;
                        Ok(())
                    })
                },
            )
            .await?;
            remove_healed(&pending)?;
            remove_healed(&path)?;
            ended.push(record);
        }
        // Also remove the already-ended records observed before recovery.
        ended.extend(require_no_running_groups(run_dir, run_id)?);
        ended.sort_by(|a, b| a.file.cmp(&b.file));
        ended.dedup_by(|a, b| a.file == b.file);
        Ok(ended)
    }
    #[cfg(not(unix))]
    {
        let _ = check;
        require_no_running_groups(run_dir, run_id)
    }
}

#[cfg(unix)]
use archon_shell::process_tree::{IdentityRecorder, Pinned, Scope, Tracker, snapshot};
#[cfg(unix)]
use std::collections::BTreeSet;

#[cfg(unix)]
type Retained = std::sync::Arc<std::sync::Mutex<BTreeSet<Pinned>>>;

#[cfg(unix)]
#[derive(Debug)]
struct Recorder {
    evidence: GroupEvidence,
    // Each tree's checkpoint retains every other tree's pins as well.
    retained: Retained,
    failure: std::sync::Arc<std::sync::Mutex<Option<crate::command::workflow_host_fault::Fault>>>,
}
#[cfg(unix)]
impl IdentityRecorder for Recorder {
    fn checkpoint(&mut self, pins: &[Pinned]) -> std::io::Result<()> {
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        retained.extend(pins);
        let result = (|| {
            #[cfg(test)]
            tests::checkpoint(pins)?;
            self.evidence.remember(
                &retained
                    .iter()
                    .map(|pin| (pin.pid, pin.start))
                    .collect::<Vec<_>>(),
            )
        })();
        if let Err(error) = &result {
            let typed = match error.raw_os_error() {
                Some(errno) => std::io::Error::from_raw_os_error(errno),
                None => std::io::Error::new(error.kind(), error.to_string()),
            };
            let error = anyhow::Error::new(typed);
            *self
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) =
                Some(crate::command::workflow_host_fault::Fault {
                    class: crate::command::workflow_host_fault::classify(&error),
                    text: crate::command::workflow_host_fault::diagnostic(&error),
                });
        }
        result
    }
}

#[cfg(unix)]
fn terminate_recorded(
    record: &mut HostCommandGroupRecord,
    retained: &Retained,
) -> anyhow::Result<()> {
    let path = record
        .file
        .as_ref()
        .context("recorded child has no record path")?;
    let table = snapshot()?;
    let pins = {
        let mut pins = retained.lock().unwrap_or_else(|error| error.into_inner());
        pins.extend(
            record
                .survivors
                .iter()
                .map(|&(pid, start)| Pinned { pid, start }),
        );
        pins.extend(record.leader_start.map(|start| Pinned {
            pid: record.pgid,
            start,
        }));
        pins.clone()
    };
    let scope = Scope {
        groups: vec![record.pgid],
        sessions: record.session.into_iter().collect(),
    };
    // A reused leader invalidates scope selectors, never explicit pinned identities.
    // Select from this complete snapshot, then signal only its pinned identities.
    let replaced = record.leader_start.is_some_and(|start| {
        table
            .processes
            .iter()
            .any(|process| process.pid == record.pgid && process.start != start)
    });
    if !replaced && record.leader_start.is_some() {
        anyhow::ensure!(
            !table.listed.contains(&record.pgid)
                || table.processes.iter().any(|p| p.pid == record.pgid),
            "cannot read the leader identity in {}; recorded-child teardown remains fenced",
            path.display()
        );
    }
    let mut protected = BTreeSet::from([0, 1]);
    let mut ancestor = std::process::id();
    while protected.insert(ancestor) {
        match table.processes.iter().find(|p| p.pid == ancestor) {
            Some(process) => ancestor = process.ppid,
            None => break,
        }
    }
    anyhow::ensure!(
        !pins.iter().any(|pin| protected.contains(&pin.pid)),
        "invalid protected process identity in {}; restore the intact host command record",
        path.display()
    );
    let pending = path.with_extension("pending");
    let mut options = std::fs::OpenOptions::new();
    use std::os::unix::fs::OpenOptionsExt;
    options
        .write(true)
        .read(true)
        .custom_flags(libc::O_NOFOLLOW);
    let file = match options.open(&pending) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            options.create_new(true).open(&pending)?
        }
        Err(error) => return Err(error.into()),
    };
    let mut checkpoint = record.clone();
    // A recovery checkpoint must retain unknown membership across executor
    // restarts, even when an adoption write fails before invalidating the marker.
    checkpoint.stalled = true;
    checkpoint.survivors_unknown = true;
    checkpoint.teardown_complete = false;
    let evidence = GroupEvidence::new(file, checkpoint);
    let failure = Default::default();
    // Persist all selected roots before any signal. A crash during recovery
    // retains descendants discovered by every tracker in this same marker.
    let mut recorder = Recorder {
        evidence: evidence.clone(),
        retained: retained.clone(),
        failure: std::sync::Arc::clone(&failure),
    };
    recorder.checkpoint(&[]).map_err(|error| {
        crate::command::workflow_host_fault::workflow(
            error.into(),
            format!("survivor checkpoint {}", pending.display()),
        )
    })?;
    for pin in pins {
        if archon_shell::process_tree::identity_of(pin.pid)? != Some(pin.start) {
            continue;
        }
        let mut tracker = Tracker::new(pin, Vec::new(), Vec::new());
        // Recovery owns no unreaped child: disable all leader/scope assumptions
        // before scanning. Descendant adoption still checks each parent's identity.
        tracker.leader_reaped();
        tracker
            .set_recorder(Box::new(Recorder {
                evidence: evidence.clone(),
                retained: retained.clone(),
                failure: std::sync::Arc::clone(&failure),
            }))
            .map_err(|error| {
                crate::command::workflow_host_fault::workflow(
                    error.into(),
                    format!("survivor checkpoint {}", pending.display()),
                )
            })?;
        kill_checkpointed(&mut tracker, retained, &failure, &pending)?;
    }
    let final_table = snapshot()?;
    anyhow::ensure!(
        replaced || scope.members_in(&final_table.processes).is_empty(),
        "recorded scope in {} has members without durable ownership identities; teardown remains fenced",
        path.display()
    );
    for pin in retained
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
    {
        anyhow::ensure!(
            archon_shell::process_tree::identity_of(pin.pid)? != Some(pin.start),
            crate::command::workflow_host_fault::transient(format!(
                "recorded identity teardown remains incomplete in {}: {pin:?}",
                path.display()
            ))
        );
    }
    evidence.complete(&[]).map_err(|error| {
        crate::command::workflow_host_fault::workflow(
            error.into(),
            format!("survivor checkpoint {}", pending.display()),
        )
    })?;
    record.stalled = false;
    record.survivors_unknown = false;
    record.survivors.clear();
    record.teardown_complete = true;
    Ok(())
}

/// Recovery cannot use the supervisor's best-effort kill: every signal here
/// requires a successful durable checkpoint, including newly adopted children.
#[cfg(unix)]
fn kill_checkpointed(
    tracker: &mut Tracker,
    retained: &Retained,
    failure: &std::sync::Mutex<Option<crate::command::workflow_host_fault::Fault>>,
    pending: &Path,
) -> anyhow::Result<()> {
    let deadline =
        std::time::Instant::now() + crate::command::workflow_host_command_supervisor::REAP_DEADLINE;
    let mut stopped = BTreeSet::new();
    loop {
        let scanned = tracker.refresh(deadline);
        retained
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(tracker.pinned());
        if let Some(error) = failure
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        {
            return Err(anyhow::Error::new(error))
                .with_context(|| format!("survivor checkpoint {}", pending.display()));
        }
        anyhow::ensure!(
            tracker.recording_failure().is_none(),
            "survivor checkpoint {}: {:?}",
            pending.display(),
            tracker.recording_failure()
        );
        let members = scanned?;
        if members.is_empty() {
            return Ok(());
        }
        let mut fresh = false;
        for pin in &members {
            if stopped.insert(*pin) {
                archon_shell::process_tree::deliver(*pin, libc::SIGSTOP);
                fresh = true;
            }
        }
        if !fresh {
            for pin in members {
                archon_shell::process_tree::deliver(pin, libc::SIGKILL);
            }
            std::thread::yield_now();
        }
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_host_command_groups_recover_tests.rs"]
mod tests;
