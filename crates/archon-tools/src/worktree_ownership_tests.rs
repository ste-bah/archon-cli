//! Unit tests for worktree ownership keys and lock-based liveness.
//!
//! These exercise the ownership primitives in isolation. The end-to-end
//! regression fixtures pinned by M4 (#184) — collision, reclaim, exit
//! resolution, resume reuse — live in `tests/worktree_tests.rs`, because they
//! need a real git repository.

use super::*;

/// Each test gets its own worktrees root. The real one is user-global and
/// shared by every archon on the machine, which is the whole reason the lock
/// exists; a test that used it would race the developer's own session.
fn temp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

#[test]
fn a_subagent_and_its_session_never_share_a_key() {
    let session = session_owner_key("sess-1");
    let subagent = subagent_owner_key("sess-1");
    assert_ne!(session, subagent);
    assert_eq!(session, "session-sess-1");
    assert_eq!(subagent, "subagent-sess-1");
}

/// The bug in one assertion: two agents in one session must not resolve to the
/// same directory.
#[test]
fn two_agents_in_one_session_get_distinct_keys() {
    let first = owner_key_for("shared-session", Some("agent-a"));
    let second = owner_key_for("shared-session", Some("agent-b"));
    let parent = owner_key_for("shared-session", None);

    assert_ne!(first, second);
    assert_ne!(first, parent);
    assert_ne!(second, parent);
}

/// `None` means the top-level agent — a real answer, not missing data.
#[test]
fn an_absent_subagent_id_selects_the_session_key() {
    assert_eq!(owner_key_for("s", None), session_owner_key("s"));
    assert_eq!(owner_key_for("s", Some("   ")), session_owner_key("s"));
    assert_eq!(owner_key_for("s", Some("")), session_owner_key("s"));
}

#[test]
fn keys_stay_single_path_components() {
    let key = owner_key_for("a/b\\c:d", None);
    assert!(
        !key.contains('/'),
        "key must not contain a separator: {key}"
    );
    assert!(
        !key.contains('\\'),
        "key must not contain a separator: {key}"
    );
    assert!(!key.contains(':'), "key must not contain a colon: {key}");
}

#[test]
fn an_unlocked_owner_reads_as_free() {
    let root = temp_root();
    assert_eq!(
        owner_liveness(root.path(), "session-nobody"),
        OwnerLiveness::Free
    );
}

#[test]
fn acquiring_marks_the_owner_as_ours_and_releasing_frees_it() {
    let root = temp_root();
    let owner = "session-acquire-release";

    assert!(acquire(root.path(), owner).expect("first acquire"));
    assert_eq!(owner_liveness(root.path(), owner), OwnerLiveness::Ours);

    release(owner);
    assert_eq!(owner_liveness(root.path(), owner), OwnerLiveness::Free);
}

/// Re-entry is a resume, not a collision: the same process taking its own lock
/// again must not report the worktree as foreign-held, or an agent could never
/// re-enter its own tree.
#[test]
fn re_acquiring_our_own_lock_is_reuse_not_failure() {
    let root = temp_root();
    let owner = "session-reentrant";

    assert!(acquire(root.path(), owner).expect("first acquire"));
    assert!(
        !acquire(root.path(), owner).expect("second acquire must succeed"),
        "re-entry should report `false` (already held), not take a new lock"
    );
    assert_eq!(owner_liveness(root.path(), owner), OwnerLiveness::Ours);

    release(owner);
}

/// The lock file sits beside the worktree directory, not inside it — a reclaim
/// calls `remove_dir_all` on the directory, and deleting the file whose lock
/// proves the reclaim is safe would defeat the check.
#[test]
fn the_lock_file_is_not_inside_the_worktree_directory() {
    let root = temp_root();
    let owner = "session-lock-placement";
    let worktree_dir = root.path().join(owner);

    assert!(!lock_path(root.path(), owner).starts_with(&worktree_dir));
    assert!(!marker_path(root.path(), owner).starts_with(&worktree_dir));
}

#[test]
fn a_refusal_can_name_the_owner_from_the_marker() {
    let root = temp_root();
    let owner = "subagent-abc";
    write_marker(root.path(), owner, "sess-9", "2026-08-15T10:00:00Z").expect("write marker");

    let described = describe_owner(root.path(), owner);
    assert!(described.contains("subagent-abc"), "got: {described}");
    assert!(described.contains("sess-9"), "got: {described}");
}

/// A missing marker must still produce a usable message rather than panicking
/// or naming nobody — an unreadable owner is exactly when the operator needs
/// the error most.
#[test]
fn describe_owner_falls_back_to_the_key_when_no_marker_exists() {
    let root = temp_root();
    let described = describe_owner(root.path(), "subagent-missing");
    assert!(described.contains("subagent-missing"), "got: {described}");
}

#[test]
fn forget_removes_both_files_and_releases_the_lock() {
    let root = temp_root();
    let owner = "session-forget";

    acquire(root.path(), owner).expect("acquire");
    write_marker(root.path(), owner, "sess", "2026-08-15T10:00:00Z").expect("marker");
    assert!(lock_path(root.path(), owner).exists());
    assert!(marker_path(root.path(), owner).exists());

    forget(root.path(), owner);

    assert!(!marker_path(root.path(), owner).exists());
    assert!(!lock_path(root.path(), owner).exists());
    assert_eq!(owner_liveness(root.path(), owner), OwnerLiveness::Free);
}

#[test]
fn read_marker_round_trips_identity() {
    let root = temp_root();
    let owner = "subagent-round-trip";
    write_marker(root.path(), owner, "sess-round", "2026-08-15T11:22:33Z").expect("write");

    let identity = read_marker(root.path(), owner).expect("marker present");
    assert_eq!(identity.owner_id, owner);
    assert_eq!(identity.session_id, "sess-round");
    assert_eq!(identity.created_at, "2026-08-15T11:22:33Z");
}
