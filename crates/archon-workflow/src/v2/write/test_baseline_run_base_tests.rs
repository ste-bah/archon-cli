//! Issue-118: the host runs a verifier's runner command itself -- at the
//! run's base commit in a throwaway worktree, and on the judged tree in
//! place -- caches only completed runs, and runs nothing but a plain runner.
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Tree, host_runnable, host_verdicts, panic_files, run_base_commit, signature};
use crate::agent_dispatch_port::{HostCommandEnv, WorkflowAgentDispatch};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::{WorkflowV2AgentAdapter, WorkflowV2CallExecution, WorkflowV2ResultStore};
use crate::write_coordinator::worktree_isolation::run_git;
use crate::{WorkflowError, WorkflowResult, WorkflowV2Result};

/// A host whose runner environment puts a fake `cargo` first on `PATH`.
pub(crate) struct FakeCargo {
    pub(crate) bin: PathBuf,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for FakeCargo {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn host_command_env(&self, _: &Path) -> HostCommandEnv {
        let path = std::env::var("PATH").unwrap_or_default();
        HostCommandEnv {
            vars: vec![("PATH".into(), format!("{}:{path}", self.bin.display()))],
            hold: None,
        }
    }
    fn baseline_test_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(20))
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        _: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        Err(WorkflowError::StageFailed("no agent runs here".into()))
    }
}

fn git(root: &Path, args: &[&str]) {
    run_git(args, root).expect("git");
}

pub(crate) fn head(root: &Path) -> String {
    String::from_utf8(run_git(&["rev-parse", "HEAD"], root).unwrap().stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// Commit `files` (created empty) to `repo`.
pub(crate) fn commit_files(repo: &Path, files: &[&str], message: &str) {
    for file in files {
        std::fs::write(repo.join(file), "// x\n").unwrap();
    }
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", message]);
}

/// A package `app` whose fake runner reads marker files: `src/old_red`
/// fails `shared::tests::old` at `src/shared_tests.rs:3:5`, with message
/// `old broke` -- or `old broke differently` when `src/old_changed` exists;
/// `src/new_red` fails `shared::tests::new`; `src/doc_red` fails a doc test
/// (a failure no test id names); a `--test` run says `old broke in the
/// other binary` when `src/other_changed` exists; `src/no_summary` fails
/// before any test runs. The base commit holds `src/old_red`; `HEAD` changes
/// nothing a test reads. Every run is counted.
pub(crate) fn world(dir: &Path) -> (PathBuf, String, FakeCargo, PathBuf) {
    let repo = dir.join("canonical");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    commit_files(
        &repo,
        &[
            "src/lib.rs",
            "src/shared.rs",
            "src/shared_tests.rs",
            "src/mine.rs",
            "src/old_red",
        ],
        "base",
    );
    let base = head(&repo);
    commit_files(&repo, &["src/landed"], "a task lands");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let counter = dir.join("runs");
    let script = format!(
        "#!/bin/sh\necho \"run $*\" >> \"{}\"\necho '     Running unittests src/lib.rs (target/debug/deps/app-0)'\n\
         if [ -f src/no_summary ]; then echo 'error[E0425]: cannot find value'; exit 101; fi\n\
         fail=0\nmsg='old broke'\n[ -f src/old_changed ] && msg='old broke differently'\n\
         case \"$*\" in *--test*) [ -f src/other_changed ] && msg='old broke in the other binary';; esac\n\
         if [ -f src/doc_red ]; then echo 'test src/lib.rs - doc (line 5) ... FAILED'; fail=$((fail+1)); fi\n\
         if [ -f src/old_red ]; then echo 'test shared::tests::old ... FAILED'; \
         echo \"thread 'shared::tests::old' (7) panicked at src/shared_tests.rs:3:5:\"; \
         echo \"$msg in $PWD/x\"; fail=$((fail+1)); fi\n\
         if [ -f src/new_red ]; then echo 'test shared::tests::new ... FAILED'; \
         echo \"thread 'shared::tests::new' (8) panicked at src/shared_tests.rs:9:5:\"; \
         echo 'new broke'; fail=$((fail+1)); fi\n\
         if [ $fail -gt 0 ]; then echo \"test result: FAILED. 1 passed; $fail failed\"; exit 101; fi\n\
         echo 'test result: ok. 2 passed'; exit 0\n",
        counter.display()
    );
    let cargo = bin.join("cargo");
    std::fs::write(&cargo, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (repo, base, FakeCargo { bin }, counter)
}

pub(crate) fn runs(counter: &Path) -> usize {
    std::fs::read_to_string(counter)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// The run directory's first event, as the live host writes it.
pub(crate) fn bind_run(store: &WorkflowV2ResultStore, head: &str) {
    let run = store.root().parent().unwrap();
    std::fs::create_dir_all(run).unwrap();
    std::fs::write(
        run.join("events.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({"seq": 1, "kind": "started", "detail": {
                "event": "repository_bound", "head": head, "drift": false}})
        ),
    )
    .unwrap();
}

#[test]
fn only_a_plain_runner_invocation_is_host_runnable() {
    assert!(host_runnable("cargo test -p archon-trading --lib"));
    assert!(host_runnable("cargo nextest run -p app --lib pine"));
    assert!(host_runnable(
        "cargo test -q -p app --features=x --test it name -- --exact --test-threads 2"
    ));
    for refused in [
        "cargo test -p app; rm -rf /",
        "cargo test $(cat x)",
        "cargo test | tee out",
        "cargo test > out",
        "cargo clippy -p app",
        "cargo build --tests",
        "cargo run --bin x -- test",
        "cargo fmt -- nextest",
        "mcp__tradingview__pine_get_errors()",
        "sh -c 'cargo test'",
        ": cargo test -p app",
        "cargo test --config build.rustc-wrapper=x -p app",
        "cargo test --manifest-path=x.toml",
        "cargo +nightly test",
        "cargo test -Zbuild-std",
        "cargo test -p app 'quoted'",
        "cargo test -p app ../escape",
        "cargo test -p app -- --logfile=Cargo.toml",
        "cargo test -p app -- --logfile Cargo.toml",
        "cargo test --unknown-option",
        "cargo test -p",
        "cargo test -p --lib",
    ] {
        assert!(!host_runnable(refused), "{refused}");
    }
}

#[test]
fn a_signature_is_the_panic_location_and_message_with_paths_normalised() {
    let a = "thread 'a::t::x' (12) panicked at src/a/t.rs:9:5:\nboom in /tmp/one/x\n";
    let b = "thread 'a::t::x' (99) panicked at src/a/t.rs:9:5:\nboom in /var/two/y\n";
    let c = "thread 'a::t::x' (12) panicked at src/a/t.rs:9:5:\nbang\n";
    assert_eq!(signature(a, "a::t::x"), signature(b, "a::t::x"));
    assert_ne!(signature(a, "a::t::x"), signature(c, "a::t::x"));
    assert_eq!(signature(a, "a::t::other"), "");
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src/a")).unwrap();
    std::fs::write(temp.path().join("src/a/t.rs"), "").unwrap();
    let old = "thread 'a::t::x' panicked at 'boom', src/a/t.rs:1:2\n";
    assert_eq!(
        panic_files(old, "a::t::x", temp.path()),
        vec!["src/a/t.rs".to_string()]
    );
    assert!(
        panic_files(
            "thread 'a::t::x' panicked at src/gone.rs:1:1:\n",
            "a::t::x",
            temp.path()
        )
        .is_empty()
    );
}

#[test]
fn the_run_base_is_the_head_the_runs_first_repository_binding_recorded() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    assert_eq!(run_base_commit(&store), None);
    bind_run(&store, "abc123");
    assert_eq!(run_base_commit(&store).as_deref(), Some("abc123"));
}

#[tokio::test]
async fn both_trees_are_run_once_the_base_in_a_removed_worktree_and_cached() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, base, host, counter) = world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let command = "cargo test -p app --lib".to_string();
    let commands = [command.clone(), "cargo test; echo x".into()];
    let at_base = host_verdicts(&store, &host, &repo, Tree::RunBase, &base, &commands).await;
    assert_eq!(runs(&counter), 1, "only the plain command ran");
    assert!(
        std::fs::read_to_string(&counter)
            .unwrap()
            .contains("--no-fail-fast"),
        "the host always runs every test binary"
    );
    assert_eq!(
        at_base[&command].failing_tests,
        vec!["shared::tests::old".to_string()]
    );
    assert!(at_base[&command].signatures["shared::tests::old"].contains("old broke"));
    assert!(
        !store
            .root()
            .join(format!("worktrees/run-base-{}", &base[..12]))
            .exists(),
        "the throwaway worktree is removed"
    );
    commit_files(&repo, &["src/new_red"], "a task breaks another test");
    let now = head(&repo);
    let judged = host_verdicts(&store, &host, &repo, Tree::Judged, &now, &commands).await;
    assert_eq!(runs(&counter), 2);
    assert_eq!(judged[&command].failing_tests.len(), 2);
    assert_eq!(
        judged[&command].signatures["shared::tests::old"],
        at_base[&command].signatures["shared::tests::old"],
        "the same failure in two directories reads alike"
    );
    for tree in [Tree::RunBase, Tree::Judged] {
        let commit = if tree == Tree::RunBase { &base } else { &now };
        host_verdicts(
            &store,
            &host,
            &repo,
            tree,
            commit,
            std::slice::from_ref(&command),
        )
        .await;
    }
    assert_eq!(runs(&counter), 2, "served from the cache");
}

#[tokio::test]
async fn no_verdict_without_a_test_summary_a_readable_base_or_the_judged_head() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, base, host, counter) = world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let command = vec!["cargo test -p app".to_string()];
    let zero = "0000000000000000000000000000000000000000";
    assert!(
        host_verdicts(&store, &host, &repo, Tree::RunBase, zero, &command)
            .await
            .is_empty()
    );
    // The checkout moved on from the commit the verifier judged.
    assert!(
        host_verdicts(&store, &host, &repo, Tree::Judged, &base, &command)
            .await
            .is_empty()
    );
    assert_eq!(runs(&counter), 0);
    // A build that fails before any test: no verdict, and never cached.
    commit_files(&repo, &["src/no_summary"], "breaks the build");
    let now = head(&repo);
    for _ in 0..2 {
        assert!(
            host_verdicts(&store, &host, &repo, Tree::Judged, &now, &command)
                .await
                .is_empty()
        );
    }
    assert_eq!(runs(&counter), 2, "run again each time, never cached");
}

#[test]
fn a_tests_file_is_resolved_at_the_commit_that_was_judged_not_the_tree_now() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, base, _, _) = world(temp.path());
    std::fs::create_dir_all(repo.join("src/shared")).unwrap();
    commit_files(
        &repo,
        &["src/shared/tests.rs"],
        "a later task moves the module",
    );
    let at = super::super::test_baseline_owner_at::test_file_at;
    let id = "shared::tests::old";
    assert_eq!(
        at(&repo, Some(&base), "cargo test -p app", id).as_deref(),
        Some("src/shared_tests.rs")
    );
    assert_eq!(
        at(&repo, Some(&head(&repo)), "cargo test -p app", id).as_deref(),
        Some("src/shared/tests.rs")
    );
    let parent = super::super::test_baseline_owner_at::parent_module_file_at;
    assert_eq!(
        parent(&repo, Some(&base), "cargo test -p app", id).as_deref(),
        Some("src/shared.rs")
    );
}

#[test]
fn a_run_is_a_verdict_only_when_every_test_binary_reported() {
    use super::args::{complete_run, harness_reported};
    let one = "     Running unittests src/lib.rs (t/a)\ntest result: FAILED. 1 failed\n";
    assert!(harness_reported(one));
    let stopped = format!("{one}     Running tests/it.rs (t/b)\n");
    assert!(
        !harness_reported(&stopped),
        "cargo stopped at the first failing binary"
    );
    assert!(!harness_reported(
        "     Running `rustc --crate-name x`\nerror: could not compile\n"
    ));
    assert!(harness_reported(
        "     Summary [   1.0s] 3 tests run: 2 passed, 1 failed\n"
    ));
    assert_eq!(
        complete_run("cargo test -p app --lib"),
        "cargo test -p app --lib --no-fail-fast"
    );
    assert_eq!(
        complete_run("cargo test -q -p app -v --lib"),
        "cargo test -p app --lib --no-fail-fast",
        "quiet and verbose runs print other headers"
    );
    assert_eq!(
        complete_run("cargo test -p app -- --exact x"),
        "cargo test -p app --no-fail-fast -- --exact x"
    );
    assert_eq!(
        complete_run("cargo nextest run --no-fail-fast"),
        "cargo nextest run --no-fail-fast"
    );
}

#[test]
fn an_assertions_left_and_right_are_part_of_its_signature() {
    let run = |right: &str| {
        format!(
            "thread 'a::t::x' (1) panicked at src/a.rs:9:5:\nassertion `left == right` failed\n  left: Healthy\n right: {right}\nnote: run with RUST_BACKTRACE=1\n"
        )
    };
    assert_ne!(
        signature(&run("Degraded"), "a::t::x"),
        signature(&run("Healthy2"), "a::t::x")
    );
    assert_eq!(
        signature(&run("Degraded"), "a::t::x"),
        signature(&run("Degraded"), "a::t::x")
    );
}

#[test]
fn the_failure_count_is_summed_over_every_summary_line() {
    use super::args::failed_count;
    let out =
        "test result: FAILED. 3 passed; 2 failed; 0 ignored\ntest result: ok. 1 passed; 0 failed\n";
    assert_eq!(failed_count(out), Some(2));
    assert_eq!(
        failed_count("     Summary [ 1s] 4 tests run: 3 passed, 1 failed\n"),
        Some(1)
    );
    assert_eq!(failed_count("no summary\n"), None);
}
