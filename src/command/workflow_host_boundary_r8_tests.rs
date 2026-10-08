//! Round 8 (#297): a cancelled call seals its staging only once its process
//! trees are torn down, and a teardown that is not confirmed leaves a
//! residue record that the next resume clears.
use super::*;
use crate::command::workflow_host_command_teardown_latch::TeardownToken;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stop {
    /// The call still owns the run when its future is dropped.
    Owned,
    OperatorPaused,
    OperatorCancelled,
}

/// A child whose tree the test tears down: the token stands in for the
/// supervisor's teardown thread and reports when the test says so.
struct TornDownLater {
    ready: tokio::sync::Notify,
    token: std::sync::Mutex<Option<TeardownToken>>,
    calls: AtomicUsize,
    /// What the second child found in its staging.
    found: std::sync::Mutex<Option<Vec<String>>>,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for TornDownLater {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let envelope = request
            .declared_write_set
            .iter()
            .find(|path| path.ends_with("gate-envelope.json"))
            .unwrap();
        let root = envelope.parent().unwrap();
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            let names = std::fs::read_dir(root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            *self.found.lock().unwrap() = Some(names);
            return Ok(SupervisedProcessOutput {
                exit_code: Some(1),
                timed_out: false,
                stdout_bytes: 0,
                stderr_bytes: 0,
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }
        *self.token.lock().unwrap() = Some(control.track_teardown());
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(envelope, b"{}").unwrap();
        self.ready.notify_one();
        std::future::pending().await
    }
}

async fn cancelled(stop: Stop) -> (Fixture, Arc<TornDownLater>) {
    let process = Arc::new(TornDownLater {
        ready: tokio::sync::Notify::new(),
        token: Default::default(),
        calls: AtomicUsize::new(0),
        found: Default::default(),
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
    if stop != Stop::Owned {
        fixture
            .store
            .with_run_lock(&fixture.run_id, |locked| {
                let mut run = locked.load_state(&fixture.run_id)?;
                if stop == Stop::OperatorPaused {
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
    // The child still runs while its tree is torn down, and writes a secret.
    std::fs::write(fixture.call_root.join("late.txt"), CANARY).unwrap();
    (fixture, process)
}

fn teardown(process: &TornDownLater, confirmed: bool) {
    let token = process.token.lock().unwrap().take().expect("tree tracked");
    if confirmed {
        token.settled(true);
    } else {
        drop(token);
    }
}

fn no_clear_secret(fixture: &Fixture) {
    if let Ok(bytes) = std::fs::read(fixture.call_root.join("late.txt")) {
        panic!("a secret written during teardown stayed: {bytes:?}");
    }
    let envelope = std::fs::read(fixture.call_root.join("gate-envelope.json")).unwrap();
    assert!(!String::from_utf8_lossy(&envelope).contains(CANARY));
}

#[tokio::test]
async fn a_cancelled_call_seals_after_its_teardown_not_before() {
    for stop in [Stop::Owned, Stop::OperatorCancelled] {
        let (fixture, process) = cancelled(stop).await;
        assert!(
            fixture.call_root.join("late.txt").exists(),
            "{stop:?}: sealing waits for the teardown"
        );
        teardown(&process, true);
        no_clear_secret(&fixture);
        assert!(events_named(&fixture, "host_command_staging_residue").is_empty());
        assert!(events_named(&fixture, "host_command_staging_pause").is_empty());
        let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
        let expected = if stop == Stop::Owned {
            RunStatus::Running
        } else {
            RunStatus::Cancelled
        };
        assert_eq!(
            status, expected,
            "{stop:?}: a confirmed teardown records nothing"
        );
    }
}

#[tokio::test]
async fn an_unconfirmed_teardown_after_an_operator_stop_records_the_residue() {
    for stop in [Stop::OperatorPaused, Stop::OperatorCancelled] {
        let (fixture, process) = cancelled(stop).await;
        teardown(&process, false);
        no_clear_secret(&fixture);
        let residue = events_named(&fixture, "host_command_staging_residue");
        assert_eq!(residue.len(), 1, "{stop:?}: {residue:?}");
        let path = fixture.call_root.display().to_string();
        assert_eq!(residue[0]["detail"]["path"], path.as_str());
        assert_eq!(residue[0]["detail"]["residue"], true);
        let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
        let expected = if stop == Stop::OperatorPaused {
            RunStatus::Paused
        } else {
            RunStatus::Cancelled
        };
        assert_eq!(status, expected, "the operator's decision is kept");
    }
}

#[tokio::test]
async fn an_unconfirmed_teardown_of_an_owned_call_pauses_and_the_resume_clears_it() {
    let (fixture, process) = cancelled(Stop::Owned).await;
    teardown(&process, false);
    no_clear_secret(&fixture);
    let pauses = events_named(&fixture, "host_command_staging_pause");
    assert_eq!(pauses.len(), 1, "{pauses:?}");
    assert_eq!(pauses[0]["detail"]["residue"], true);
    // A survivor writes again after the seal; the resume clears it first.
    std::fs::write(fixture.call_root.join("survivor.txt"), CANARY).unwrap();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    run.status = RunStatus::Running;
    fixture.store.save_state(&run).unwrap();
    let result = fixture
        .executor
        .execute(request(), Some(run.generation))
        .await;
    assert!(result.is_ok(), "{result:?}");
    let found = process.found.lock().unwrap().take().expect("child ran");
    assert!(
        found.is_empty(),
        "the resumed child saw old staging: {found:?}"
    );
}

/// The real supervisor: a dropped call's tree reports to the latch only
/// after its processes are gone, so the sealing it gates runs after them.
#[tokio::test]
async fn a_dropped_supervisor_reports_its_teardown_only_once_the_tree_is_gone() {
    use crate::command::workflow_host_command_supervisor::supervise_process_group;
    use crate::command::workflow_host_command_teardown_latch::TeardownLatch;
    let dir = tempfile::tempdir().unwrap();
    let request = ResolvedHostCommand {
        command_id: "fixture".into(),
        program: "sh".into(),
        args: vec![
            "-c".into(),
            "trap '' TERM; echo $$ > pid.tmp; mv pid.tmp pid; exec sleep 30".into(),
        ],
        cwd: dir.path().into(),
        environment: crate::command::workflow_host_environment::process_environment(),
        stdin: None,
        timeout_secs: 60,
        max_stdout_bytes: 4096,
        max_stderr_bytes: 4096,
        declared_write_set: vec![],
        remediation_scopes: Default::default(),
    };
    let latch = TeardownLatch::default();
    let (control, _handle) = HostCommandControl::tracked(latch.clone());
    let mut work = Box::pin(supervise_process_group(request, control, None));
    let pid_file = dir.path().join("pid");
    let began = std::time::Instant::now();
    while !pid_file.exists() {
        assert!(began.elapsed() < std::time::Duration::from_secs(10));
        let tick = tokio::time::timeout(std::time::Duration::from_millis(20), &mut work).await;
        assert!(tick.is_err(), "the child ended early: {tick:?}");
    }
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(latch.pending(), "a running tree is tracked");
    drop(work);
    let (sender, receiver) = std::sync::mpsc::channel();
    latch.after_teardown(Box::new(move |confirmed| {
        // SAFETY: signal 0 only checks whether the pid exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        sender.send((confirmed, alive)).unwrap();
    }));
    let (confirmed, alive) = tokio::task::spawn_blocking(move || {
        receiver
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the teardown reported")
    })
    .await
    .unwrap();
    assert!(confirmed, "the teardown confirmed the tree empty");
    assert!(!alive, "the step ran while the child still existed");
}

/// A clean child that also leaves what a tool it runs can leave in staging:
/// a socket (an agent's), and a FIFO, nested and at the call root.
struct LeavesSpecialFiles;

#[async_trait::async_trait]
impl HostCommandProcessAdapter for LeavesSpecialFiles {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let root = request
            .declared_write_set
            .iter()
            .find(|path| path.ends_with("gate-envelope.json"))
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let output = SecretPrintingProcess(Child::Clean)
            .execute(request, control)
            .await?;
        std::fs::create_dir_all(root.join("gnupg")).unwrap();
        // Bound at a short path (the address length limit), then moved in.
        let short = tempfile::tempdir().unwrap();
        let listener = std::os::unix::net::UnixListener::bind(short.path().join("s")).unwrap();
        std::fs::rename(short.path().join("s"), root.join("gnupg").join("S.agent")).unwrap();
        drop(listener);
        let fifo = std::ffi::CString::new(root.join("pipe").to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        Ok(output)
    }
}

#[tokio::test]
async fn a_socket_or_fifo_left_in_staging_never_pauses_the_call() {
    let fixture = fixture(Arc::new(LeavesSpecialFiles));
    let result = fixture
        .executor
        .execute(request(), Some(fixture.generation))
        .await;
    let pauses = events_named(&fixture, "host_command_staging_pause");
    assert!(pauses.is_empty(), "{pauses:?}");
    let result = result.expect("the special files are removed, not a pause");
    assert!(result.publication_receipt.is_some(), "{result:?}");
    assert!(!fixture.call_root.join("gnupg").join("S.agent").exists());
    assert!(std::fs::symlink_metadata(fixture.call_root.join("pipe")).is_err());
    let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
    assert_eq!(status, RunStatus::Running);
}

mod finish {
    //! Published sources leave staging through the anchor, and a cleanup
    //! failure after a committed publication keeps the receipt (#297 r8).
    use super::*;
    use crate::command::workflow_host_command_exec::finish::{remove_published, settle_cleanup};
    use crate::command::workflow_host_command_publish::{CommandStaging, prepare_staging};
    use crate::command::workflow_host_secrets::HostSecrets;
    use crate::command::workflow_host_staging_pause::StagingPause;

    fn staging(fixture: &Fixture) -> (CommandStaging, StagingPause) {
        let run_root = fixture.store.run_dir(&fixture.run_id);
        let project = fixture._temp.path().join("project");
        let pause =
            StagingPause::new(&project, &run_root, fixture.generation, "call-8", "cmd").unwrap();
        (prepare_staging(&run_root, "call-8").unwrap(), pause)
    }

    fn receipt(paths: &[&str]) -> archon_workflow::PublicationReceiptV1 {
        archon_workflow::PublicationReceiptV1 {
            schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
            call_id: "call-8".into(),
            command_id: "cmd".into(),
            entries: paths
                .iter()
                .map(|path| archon_workflow::PublishedArtifactReceipt {
                    relative_path: (*path).into(),
                    destination_path: format!("/live/{path}"),
                    byte_len: 1,
                    blake3: "digest".into(),
                    prior_blake3: None,
                })
                .collect(),
            committed_at: "now".into(),
        }
    }

    fn secrets() -> HostSecrets {
        HostSecrets::of(
            &context(tempfile::tempdir().unwrap().path()),
            &Default::default(),
        )
    }

    fn residue_paths(fixture: &Fixture) -> Vec<String> {
        events_named(fixture, "host_command_staging_residue")
            .iter()
            .map(|event| event["detail"]["path"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn published_sources_are_removed_and_a_failure_is_recorded_not_dropped() {
        let fixture = fixture(Arc::new(SecretPrintingProcess(Child::Clean)));
        let (staging, pause) = staging(&fixture);
        let root = staging.root.clone();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.md"), b"outside").unwrap();
        for dir in ["locked", "swapped"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("ok.md"), b"x").unwrap();
        std::fs::write(root.join("locked").join("out.md"), b"x").unwrap();
        set_mode(&root.join("locked"), 0o555);
        std::fs::remove_dir(root.join("swapped")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("swapped")).unwrap();
        remove_published(
            &staging,
            &receipt(&["ok.md", "locked/out.md", "swapped/keep.md", "gone.md"]),
            &pause,
            &secrets(),
        );
        set_mode(&root.join("locked"), 0o755);
        assert!(!root.join("ok.md").exists(), "removed through the anchor");
        assert_eq!(
            std::fs::read(outside.path().join("keep.md")).unwrap(),
            b"outside",
            "a swapped parent is never followed"
        );
        let mut found = residue_paths(&fixture);
        found.sort();
        let expected = [root.join("locked/out.md"), root.join("swapped/keep.md")];
        assert_eq!(
            found,
            expected.map(|path| path.display().to_string()).to_vec(),
            "every source left behind is recorded, an absent one is not"
        );
    }

    fn published(receipt: Option<archon_workflow::PublicationReceiptV1>) -> HostCommandResult {
        HostCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: None,
            publication_receipt: receipt,
            subjects: Vec::new(),
            postcondition: None,
        }
    }

    #[test]
    fn a_cleanup_failure_after_a_commit_keeps_the_receipt_and_records_it() {
        let fixture = fixture(Arc::new(SecretPrintingProcess(Child::Clean)));
        let (staging, pause) = staging(&fixture);
        let failed = || Err(WorkflowError::ControlPaused("sealing failed".into()));
        let kept = settle_cleanup(
            Ok(published(Some(receipt(&["a.md"])))),
            failed(),
            &staging.root,
            &pause,
            &secrets(),
        )
        .expect("a committed publication keeps its receipt");
        assert_eq!(kept.publication_receipt, Some(receipt(&["a.md"])));
        assert_eq!(
            residue_paths(&fixture),
            vec![staging.root.display().to_string()]
        );
        // Nothing committed: the sealing failure is the call's outcome.
        let uncommitted = settle_cleanup(
            Ok(published(None)),
            failed(),
            &staging.root,
            &pause,
            &secrets(),
        );
        assert!(matches!(uncommitted, Err(WorkflowError::ControlPaused(_))));
        // An earlier failure stays the outcome when sealing succeeds.
        let earlier = settle_cleanup(
            Err(WorkflowError::StageFailed("audit".into())),
            Ok(()),
            &staging.root,
            &pause,
            &secrets(),
        );
        assert!(matches!(earlier, Err(WorkflowError::StageFailed(_))));
        assert_eq!(residue_paths(&fixture).len(), 1, "only the commit records");
    }
}
