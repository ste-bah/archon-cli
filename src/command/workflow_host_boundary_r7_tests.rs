//! Round 7 (#297): every host mutation of child-writable staging stays inside
//! the call's tree verified at creation, a sealing I/O failure pauses the run,
//! and a cancelled call whose cleanup fails leaves a durable pause naming it.
use super::*;
use archon_workflow::{RunStatus, WorkflowError, WorkflowStore};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "macos")]
#[path = "workflow_host_boundary_r7_cancel_tests.rs"]
mod cancel;

#[path = "workflow_host_boundary_r8_tests.rs"]
mod r8;

pub(super) struct Fixture {
    pub(super) _temp: tempfile::TempDir,
    pub(super) store: WorkflowStore,
    pub(super) run_id: String,
    pub(super) generation: u64,
    pub(super) executor: FixedHostCommandExecutor,
    pub(super) call_root: PathBuf,
}

pub(super) fn request() -> HostCommandRequest {
    HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap()
}

pub(super) fn fixture(process: Arc<dyn HostCommandProcessAdapter>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    context.freeze_provider_environment = [("ANTHROPIC_API_KEY".into(), CANARY.into())].into();
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = WorkflowStore::project(&context.project_root);
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-staging-anchor".into(),
            task: "seal".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root.clone(),
        process,
    );
    let call_id = executor.call_identity(&request()).unwrap();
    Fixture {
        _temp: temp,
        store,
        run_id: run.id,
        generation: run.generation,
        executor,
        call_root: run_root.join("host-command-staging").join(call_id),
    }
}

pub(super) fn events_named(fixture: &Fixture, name: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fixture.store.events_path(&fixture.run_id))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["detail"]["event"] == name)
        .collect()
}

/// The run paused (not failed) and one durable pause names the staging path.
pub(super) fn assert_staging_pause(fixture: &Fixture, result: &WorkflowResult<HostCommandResult>) {
    let Err(WorkflowError::ControlPaused(message)) = result else {
        panic!("a staging I/O failure pauses, never fails: {result:?}");
    };
    let path = fixture.call_root.display().to_string();
    assert!(message.contains(&path), "{message}");
    assert!(!message.contains(CANARY), "{message}");
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    let pauses = events_named(fixture, "host_command_staging_pause");
    assert_eq!(pauses.len(), 1, "{pauses:?}");
    assert_eq!(pauses[0]["kind"], "paused");
    assert_eq!(pauses[0]["detail"]["path"], path.as_str());
    assert!(pauses[0]["detail"]["call_id"].as_str().is_some());
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tamper {
    /// A sealing I/O failure, then `host-command-staging` swapped for a link.
    AncestorSealFails,
    /// A valid call, then `host-command-staging` swapped for a link.
    AncestorSealOk,
    /// A valid call, then the call directory swapped for a link.
    CallDir,
    /// An operational ending, then the ancestor swap before the retry.
    AncestorThenRetry,
    /// A valid call whose envelope the child made unreadable.
    LockedEnvelope,
}

struct TamperProcess {
    tamper: Tamper,
    outside: PathBuf,
    before: std::sync::Mutex<Option<Snapshot>>,
    calls: AtomicUsize,
}

type Snapshot = BTreeMap<PathBuf, (u32, Option<Vec<u8>>)>;

#[async_trait::async_trait]
impl HostCommandProcessAdapter for TamperProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            return Ok(SupervisedProcessOutput {
                exit_code: Some(1),
                timed_out: false,
                stdout_bytes: 0,
                stderr_bytes: 0,
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }
        let envelope = request
            .declared_write_set
            .iter()
            .find(|path| path.ends_with("gate-envelope.json"))
            .unwrap()
            .clone();
        let child = if self.tamper == Tamper::AncestorSealFails {
            Child::UnreadableNestedStaging
        } else {
            Child::Published
        };
        let mut output = SecretPrintingProcess(child)
            .execute(request, control)
            .await?;
        let root = envelope.parent().unwrap();
        let staging = root.parent().unwrap();
        outside_tree(&self.outside, root.file_name().unwrap());
        *self.before.lock().unwrap() = Some(snapshot(&self.outside));
        match self.tamper {
            Tamper::AncestorSealFails | Tamper::AncestorSealOk | Tamper::AncestorThenRetry => {
                std::fs::rename(
                    staging,
                    staging.with_file_name("host-command-staging.moved"),
                )
                .unwrap();
                symlink(&self.outside, staging);
            }
            Tamper::CallDir => {
                std::fs::rename(root, root.with_file_name("moved-call")).unwrap();
                symlink(&self.outside.join(root.file_name().unwrap()), root);
            }
            Tamper::LockedEnvelope => set_mode(&envelope, 0),
        }
        output.timed_out = self.tamper == Tamper::AncestorThenRetry;
        Ok(output)
    }
}

fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

pub(super) fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A directory the host does not own, holding the names a staging tree has.
fn outside_tree(outside: &Path, call: &std::ffi::OsStr) {
    let tree = outside.join(call);
    std::fs::create_dir_all(tree.join("locked")).unwrap();
    std::fs::write(tree.join("gate-envelope.json"), b"{\"outside\": true}").unwrap();
    std::fs::write(tree.join("gate-envelope.outside.tmp"), b"outside temp").unwrap();
    std::fs::write(tree.join("secret.txt"), CANARY).unwrap();
    std::fs::write(tree.join("locked").join("keep.txt"), b"outside keep").unwrap();
    set_mode(&tree.join("locked"), 0o555);
    set_mode(&tree.join("gate-envelope.json"), 0o644);
}

/// Every entry under `root`: its mode and, for a file, its bytes.
fn snapshot(root: &Path) -> Snapshot {
    let mut found = BTreeMap::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let mode = std::os::unix::fs::PermissionsExt::mode(&metadata.permissions());
        let bytes = metadata.is_file().then(|| std::fs::read(&path).unwrap());
        if metadata.is_dir() {
            found.extend(snapshot(&path));
        }
        found.insert(path, (mode, bytes));
    }
    found
}

async fn tampered(tamper: Tamper) -> (Fixture, WorkflowResult<HostCommandResult>) {
    let outside = tempfile::tempdir().unwrap();
    let process = Arc::new(TamperProcess {
        tamper,
        outside: outside.path().into(),
        before: Default::default(),
        calls: AtomicUsize::new(0),
    });
    let fixture = fixture(process.clone());
    let result = fixture
        .executor
        .execute(request(), Some(fixture.generation))
        .await;
    let before = process.before.lock().unwrap().take().expect("child ran");
    let call = fixture.call_root.file_name().unwrap();
    let after = snapshot(outside.path());
    let locked = outside.path().join(call).join("locked");
    if locked.exists() {
        set_mode(&locked, 0o755);
    }
    assert_eq!(
        after, before,
        "{tamper:?}: a host mutation followed a child's link out of the staging tree ({result:?})"
    );
    (fixture, result)
}

#[tokio::test]
async fn cleanup_after_an_ancestor_swap_stays_in_the_verified_tree_and_pauses() {
    let (fixture, result) = tampered(Tamper::AncestorSealFails).await;
    let moved = fixture
        .call_root
        .parent()
        .unwrap()
        .with_file_name("host-command-staging.moved")
        .join(fixture.call_root.file_name().unwrap());
    assert!(!moved.exists(), "the verified call tree is removed");
    assert_staging_pause(&fixture, &result);
}

#[tokio::test]
async fn sealing_after_an_ancestor_swap_never_rewrites_or_deletes_outside_files() {
    let (_fixture, result) = tampered(Tamper::AncestorSealOk).await;
    assert!(
        result.is_err(),
        "a replaced staging tree is never published"
    );
}

#[tokio::test]
async fn sealing_after_a_call_directory_swap_never_follows_the_link() {
    let (fixture, result) = tampered(Tamper::CallDir).await;
    assert!(
        std::fs::symlink_metadata(&fixture.call_root).is_err(),
        "the child's link is removed, not followed"
    );
    assert_staging_pause(&fixture, &result);
}

#[tokio::test]
async fn a_retry_after_an_ancestor_swap_never_clears_the_outside_tree() {
    let (fixture, result) = tampered(Tamper::AncestorThenRetry).await;
    assert_staging_pause(&fixture, &result);
}

/// Finding 2: a sealing I/O failure whose cleanup succeeded still pauses.
#[tokio::test]
async fn a_sealing_io_failure_with_successful_cleanup_pauses_the_run() {
    for child in [
        Child::UnreadableStaging,
        Child::UnreadableNestedStaging,
        Child::UnreadableDeepStaging,
    ] {
        let fixture = fixture(Arc::new(SecretPrintingProcess(child)));
        let result = fixture
            .executor
            .execute(request(), Some(fixture.generation))
            .await;
        assert!(!fixture.call_root.exists(), "staging removed");
        assert_staging_pause(&fixture, &result);
    }
    let (fixture, result) = tampered(Tamper::LockedEnvelope).await;
    assert!(!fixture.call_root.exists(), "staging removed");
    assert_staging_pause(&fixture, &result);
}
