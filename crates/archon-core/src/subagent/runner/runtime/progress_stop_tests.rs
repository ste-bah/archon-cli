use super::*;
use std::path::Path;
use std::sync::Arc;

fn state(tree: u64, writes: u64) -> TreeState {
    TreeState {
        tree: Some(tree),
        writes,
    }
}

#[test]
fn an_unchanged_tree_is_stopped_after_the_stall_rounds() {
    let mut stop = ProgressStop::default();
    assert!(
        stop.observe(state(1, 0)).is_none(),
        "the first round sets the baseline"
    );
    for _ in 1..STALL_ROUNDS {
        assert!(stop.observe(state(1, 0)).is_none());
    }
    assert!(stop.observe(state(1, 0)).is_some());
}

#[test]
fn a_changing_tree_or_a_new_write_restarts_the_count() {
    let mut stop = ProgressStop::default();
    stop.observe(state(1, 0));
    for _ in 1..STALL_ROUNDS {
        stop.observe(state(1, 0));
    }
    assert!(stop.observe(state(2, 0)).is_none(), "the tree moved");
    for _ in 1..STALL_ROUNDS {
        assert!(stop.observe(state(2, 0)).is_none());
    }
    assert!(stop.observe(state(2, 1)).is_none(), "a write was recorded");
}

/// Issue-213 C2: A -> B -> A -> B -> A is three returns to a state the tree
/// had left, with nothing new reached: a loop, whatever the answers.
#[test]
fn a_tree_that_keeps_returning_to_a_state_it_left_is_an_oscillation() {
    let mut osc = Oscillation::default();
    let fired: Vec<bool> = [1, 2, 1, 2, 1].map(|s| osc.observe(Some(s), true)).into();
    assert_eq!(fired, [false, false, false, false, true]);
    let mut osc = Oscillation::default();
    let fired: Vec<bool> = [1, 2, 3, 1, 2, 3]
        .map(|s| osc.observe(Some(s), true))
        .into();
    assert_eq!(fired, [false, false, false, false, false, true]);
}

/// One reverted experiment, a second try, unchanged rounds between moves, a
/// new state between returns, and a return in a round that wrote a new path
/// are all ordinary work.
#[test]
fn a_revert_a_new_state_or_a_new_path_is_not_an_oscillation() {
    let mut osc = Oscillation::default();
    for state in [1, 1, 2, 2, 2, 1, 1, 2, 3, 2, 3, 4, 5] {
        assert!(!osc.observe(Some(state), true), "fired at {state}");
    }
    let mut osc = Oscillation::default();
    for _ in 0..10 {
        assert!(!osc.observe(None, true));
    }
    let mut osc = Oscillation::default();
    for state in [1, 2, 1, 2, 1, 2, 1] {
        assert!(!osc.observe(Some(state), false), "a new path broke the run");
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "{args:?}: {out:?}");
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@example.invalid"]);
    git(dir.path(), &["config", "user.name", "t"]);
    std::fs::write(dir.path().join("f.txt"), "a\n").expect("write");
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "base"]);
    dir
}

/// The real digest: a file toggled between two contents puts the tree in the
/// same two states.
#[tokio::test]
async fn a_toggled_file_repeats_its_tree_digest() {
    let dir = repo();
    let mut digests = Vec::new();
    for content in ["b\n", "a\n", "b\n"] {
        std::fs::write(dir.path().join("f.txt"), content).expect("write");
        digests.push(tree_digest(dir.path()).await.expect("digest"));
    }
    assert_eq!(digests[0], digests[2]);
    assert_ne!(digests[0], digests[1]);
}

/// Issue-213 C2 (review): `git status` names an untracked file but not its
/// contents. A session growing a new file while a tracked lockfile flips back
/// and forth is moving: every state it passes through must differ.
#[tokio::test]
async fn a_growing_new_file_beside_a_toggling_lockfile_never_repeats_a_state() {
    let dir = repo();
    let mut digests = Vec::new();
    let mut grown = String::new();
    for (round, lock) in ["b\n", "a\n", "b\n", "a\n", "b\n"].iter().enumerate() {
        grown.push_str(&format!("line {round}\n"));
        std::fs::write(dir.path().join("new.txt"), &grown).expect("grow");
        std::fs::write(dir.path().join("f.txt"), lock).expect("toggle");
        digests.push(tree_digest(dir.path()).await.expect("digest"));
    }
    let mut unique = digests.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), digests.len(), "{digests:?}");
}

#[tokio::test]
async fn a_directory_outside_any_repository_has_no_tree_digest() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(tree_digest(dir.path()).await, None);
}

fn session(
    dir: &Path,
    agent: &str,
    guard: Option<archon_tools::workflow_read_guard::WorkflowReadGuard>,
) -> archon_tools::tool::ToolContext {
    archon_tools::tool::ToolContext {
        working_dir: dir.to_path_buf(),
        subagent_id: Some(agent.to_string()),
        workflow_read_guard: guard.map(Arc::new),
        ..archon_tools::tool::ToolContext::default()
    }
}

/// Toggle `f.txt` through the session's write tool, one round each, and
/// return the round at which the session was cut, if any.
async fn toggle(ctx: &archon_tools::tool::ToolContext, rounds: usize) -> Option<usize> {
    let agent = ctx.subagent_id.clone().unwrap();
    let file = ctx.working_dir.join("f.txt");
    let mut stop = ProgressStop::default();
    for round in 0..rounds {
        std::fs::write(&file, if round % 2 == 0 { "b\n" } else { "a\n" }).expect("write");
        archon_tools::session_progress::note_touched(&agent, &file);
        if stop
            .after_round(ctx, RoundActivity::default())
            .await
            .is_some()
        {
            return Some(round);
        }
    }
    None
}

/// Issue-213 C2 (review): through `after_round`. A write-capable workflow
/// session toggling a file is cut; an interactive one, and a read-only or
/// verifier one, are not.
#[tokio::test]
async fn only_a_write_capable_workflow_session_is_cut_for_oscillating() {
    use archon_tools::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
    let dir = repo();
    let writer = session(
        dir.path(),
        "osc-test-writer",
        Some(WorkflowReadGuard::new(40, 20, false, false)),
    );
    assert_eq!(toggle(&writer, 8).await, Some(4), "cut at the third return");
    let interactive = session(dir.path(), "osc-test-interactive", None);
    assert_eq!(toggle(&interactive, 8).await, None);
    let verifier = session(
        dir.path(),
        "osc-test-verifier",
        Some(WorkflowReadGuard::shell_only(
            &WorkflowReadGuardSettings::default(),
        )),
    );
    assert_eq!(
        toggle(&verifier, 8).await,
        None,
        "read-only sessions are exempt"
    );
}

/// Issue-213 C2 (review): a round with no write and no shell call cannot have
/// moved the tree, and costs no git.
#[tokio::test]
async fn a_reading_round_runs_no_git() {
    let dir = repo();
    let ctx = session(
        dir.path(),
        "osc-test-reader",
        Some(archon_tools::workflow_read_guard::WorkflowReadGuard::new(
            40, 20, false, false,
        )),
    );
    let mut stop = ProgressStop::default();
    for _ in 0..5 {
        assert!(
            stop.after_round(&ctx, RoundActivity::default())
                .await
                .is_none()
        );
    }
    assert_eq!(stop.probes, 0);
    let shell = RoundActivity::of(["Read", "Bash"]);
    assert!(shell.may_change_tree);
    assert!(stop.after_round(&ctx, shell).await.is_none());
    assert_eq!(stop.probes, 1, "a shell round is probed");
}

#[tokio::test]
async fn an_interactive_session_is_never_stopped() {
    let ctx = archon_tools::tool::ToolContext::default();
    let mut stop = ProgressStop::default();
    assert!(
        stop.after_round(&ctx, RoundActivity::default())
            .await
            .is_none()
    );
}

/// Issue-213 C2 (review): through `after_round`. A write-capable session that
/// grows a new file every round while a shell step regenerates a tracked
/// lockfile back and forth is working, and is never cut.
#[tokio::test]
async fn a_growing_new_file_beside_a_toggling_lockfile_is_not_cut() {
    let dir = repo();
    let ctx = session(
        dir.path(),
        "osc-test-grower",
        Some(archon_tools::workflow_read_guard::WorkflowReadGuard::new(
            40, 20, false, false,
        )),
    );
    let agent = "osc-test-grower";
    let grown_file = dir.path().join("new.txt");
    let mut grown = String::new();
    let mut stop = ProgressStop::default();
    for round in 0..10 {
        grown.push_str(&format!("line {round}\n"));
        std::fs::write(&grown_file, &grown).expect("grow");
        archon_tools::session_progress::note_touched(agent, &grown_file);
        let lock = if round % 2 == 0 { "b\n" } else { "a\n" };
        std::fs::write(dir.path().join("f.txt"), lock).expect("regenerate");
        let shell = RoundActivity::of(["Bash"]);
        assert!(
            stop.after_round(&ctx, shell).await.is_none(),
            "cut at round {round}"
        );
    }
}

/// Issue-213 C2 (review): an untracked link to a directory, a dangling link
/// and a nested repository made `hash-object` fail and switched detection off
/// for the session. Each is fingerprinted on its own now, and a regular
/// untracked file beside them still moves the digest.
#[cfg(unix)]
#[tokio::test]
async fn odd_untracked_entries_never_disable_the_digest() {
    let dir = repo();
    let root = dir.path();
    std::fs::create_dir_all(root.join("realdir")).unwrap();
    std::fs::write(root.join("realdir/x"), "x").unwrap();
    std::os::unix::fs::symlink(root.join("realdir"), root.join("dirlink")).unwrap();
    std::os::unix::fs::symlink(root.join("missing"), root.join("dangling")).unwrap();
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-q"]);
    std::fs::write(root.join("notes.txt"), "one").unwrap();
    let first = tree_digest(root)
        .await
        .expect("a digest despite odd entries");
    std::fs::write(root.join("notes.txt"), "two").unwrap();
    let second = tree_digest(root).await.expect("still a digest");
    assert_ne!(first, second, "the regular untracked file is seen");
}

/// Issue-213 C2 (review): content is read only within the bounds; past them a
/// file still counts, by its size.
#[test]
fn the_untracked_fingerprint_is_bounded_and_still_sees_capped_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), "aaaa").unwrap();
    std::fs::write(dir.path().join("b"), "bbbb").unwrap();
    let caps = untracked::Caps {
        files: 1,
        bytes: 1024,
    };
    let listed = b"a\0b\0";
    let first = untracked::fingerprint(dir.path(), listed, caps);
    assert_eq!(first, untracked::fingerprint(dir.path(), listed, caps));
    std::fs::write(dir.path().join("b"), "bbbbbb").unwrap();
    assert_ne!(first, untracked::fingerprint(dir.path(), listed, caps));
}

/// Issue-213 C2 (review): a round calling a tool not known to be read-only (an
/// MCP tool, a child agent) may have moved the tree, and is probed.
#[tokio::test]
async fn a_round_with_an_unknown_tool_is_probed() {
    assert!(!RoundActivity::of(["Read", "Grep", "Glob"]).may_change_tree);
    for tool in ["mcp__store__put", "Agent", "Bash"] {
        assert!(RoundActivity::of([tool]).may_change_tree, "{tool}");
    }
    let dir = repo();
    let ctx = session(
        dir.path(),
        "osc-test-mcp",
        Some(archon_tools::workflow_read_guard::WorkflowReadGuard::new(
            40, 20, false, false,
        )),
    );
    let mut stop = ProgressStop::default();
    let _ = stop
        .after_round(&ctx, RoundActivity::of(["mcp__store__put"]))
        .await;
    assert_eq!(stop.probes, 1);
}

/// Issue-213 C2 (review): a round that also wrote an IGNORED file moved
/// something the digest cannot see, so its return to an old digest is not
/// counted as a loop.
#[tokio::test]
async fn a_return_beside_an_unseen_write_is_not_counted() {
    let dir = repo();
    std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
    git(dir.path(), &["add", ".gitignore"]);
    git(dir.path(), &["commit", "-qm", "ignore"]);
    std::fs::create_dir_all(dir.path().join("build")).unwrap();
    let agent = "osc-test-unseen";
    let ctx = session(
        dir.path(),
        agent,
        Some(archon_tools::workflow_read_guard::WorkflowReadGuard::new(
            40, 20, false, false,
        )),
    );
    let tracked = dir.path().join("f.txt");
    let log = dir.path().join("build/out.log");
    let mut stop = ProgressStop::default();
    for round in 0..8 {
        std::fs::write(&tracked, if round % 2 == 0 { "b\n" } else { "a\n" }).unwrap();
        archon_tools::session_progress::note_touched(agent, &tracked);
        std::fs::write(&log, format!("round {round}\n")).unwrap();
        archon_tools::session_progress::note_touched(agent, &log);
        assert!(
            stop.after_round(&ctx, RoundActivity::default())
                .await
                .is_none(),
            "cut at round {round}"
        );
    }
}
