//! Finding 3 (#297 round 7): a cancelled call whose sealing and staging
//! removal both fail leaves a durable record naming the path, and the next
//! resume pauses on it instead of failing. An immutable file (`chflags
//! uchg`) is the removal failure an unprivileged test can cause; Linux needs
//! root for the equivalent, so these run on macOS.
use super::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Before {
    /// The call is still the run's owner when it is cancelled.
    Owned,
    /// An operator paused the run first.
    OperatorPaused,
    /// An operator cancelled the run first.
    OperatorCancelled,
}

struct PendingProcess {
    ready: tokio::sync::Notify,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for PendingProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        _: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let envelope = request
                .declared_write_set
                .iter()
                .find(|path| path.ends_with("gate-envelope.json"))
                .unwrap();
            let locked = envelope.parent().unwrap().join("locked");
            std::fs::create_dir_all(&locked).unwrap();
            std::fs::write(envelope, b"{}").unwrap();
            let artifact = locked.join("secret-artifact.txt");
            std::fs::write(&artifact, CANARY).unwrap();
            immutable(&artifact, true);
            set_mode(&locked, 0);
            self.ready.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(SupervisedProcessOutput {
            exit_code: Some(1),
            timed_out: false,
            stdout_bytes: 0,
            stderr_bytes: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

fn immutable(path: &Path, on: bool) {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let flags = if on { libc::UF_IMMUTABLE } else { 0 };
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::chflags(path.as_ptr(), flags) }, 0);
}

fn release(fixture: &Fixture) {
    let locked = fixture.call_root.join("locked");
    if locked.exists() {
        set_mode(&locked, 0o755);
        let artifact = locked.join("secret-artifact.txt");
        if artifact.exists() {
            immutable(&artifact, false);
        }
    }
}

async fn cancelled(before: Before) -> (Fixture, Arc<PendingProcess>) {
    let process = Arc::new(PendingProcess {
        ready: tokio::sync::Notify::new(),
        calls: AtomicUsize::new(0),
    });
    let fixture = fixture(process.clone());
    let mut call = Box::pin(
        fixture
            .executor
            .execute(request(), Some(fixture.generation)),
    );
    tokio::select! {
        result = &mut call => panic!("the pending child returned: {result:?}"),
        () = process.ready.notified() => {}
    }
    if before != Before::Owned {
        fixture
            .store
            .with_run_lock(&fixture.run_id, |locked| {
                let mut run = locked.load_state(&fixture.run_id)?;
                if before == Before::OperatorPaused {
                    archon_workflow::control_pause::apply_pause(&mut run);
                } else {
                    run.status = RunStatus::Cancelled;
                    run.generation += 1;
                }
                locked.save_state(&run)
            })
            .unwrap();
    }
    drop(call);
    (fixture, process)
}

#[tokio::test]
async fn a_cancelled_call_whose_cleanup_fails_pauses_the_run_naming_the_path() {
    let (fixture, _process) = cancelled(Before::Owned).await;
    let path = fixture.call_root.display().to_string();
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    let pauses = events_named(&fixture, "host_command_staging_pause");
    release(&fixture);
    assert_eq!(run.status, RunStatus::Paused, "{pauses:?}");
    assert_eq!(pauses.len(), 1, "{pauses:?}");
    assert_eq!(pauses[0]["kind"], "paused");
    assert_eq!(pauses[0]["detail"]["path"], path.as_str());
    assert_eq!(pauses[0]["detail"]["residue"], true);
}

#[tokio::test]
async fn a_cancelled_call_after_an_operator_stop_still_records_the_residue() {
    for before in [Before::OperatorPaused, Before::OperatorCancelled] {
        let (fixture, _process) = cancelled(before).await;
        let path = fixture.call_root.display().to_string();
        let residue = events_named(&fixture, "host_command_staging_residue");
        let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
        release(&fixture);
        assert_eq!(residue.len(), 1, "{before:?}: {residue:?}");
        assert_eq!(residue[0]["detail"]["path"], path.as_str(), "{before:?}");
        let expected = if before == Before::OperatorPaused {
            RunStatus::Paused
        } else {
            RunStatus::Cancelled
        };
        assert_eq!(status, expected, "the operator's decision is kept");
    }
}

#[tokio::test]
async fn the_next_resume_pauses_on_the_residue_instead_of_failing() {
    let (fixture, process) = cancelled(Before::Owned).await;
    // The operator resumes: the run runs again under the paused generation.
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Running;
    fixture.store.save_state(&run).unwrap();
    let result = fixture
        .executor
        .execute(request(), Some(run.generation))
        .await;
    let pauses = events_named(&fixture, "host_command_staging_pause");
    let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
    release(&fixture);
    assert_eq!(process.calls.load(Ordering::SeqCst), 1, "no child ran");
    let Err(WorkflowError::ControlPaused(message)) = &result else {
        panic!("residue from a cancelled call pauses the resume: {result:?}");
    };
    assert!(
        message.contains(&fixture.call_root.display().to_string()),
        "{message}"
    );
    assert_eq!(status, RunStatus::Paused);
    assert_eq!(pauses.len(), 2, "{pauses:?}");
}
