use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn lock_path_is_exact() {
    let root = Path::new("/repo/x");
    let hex = blake3::hash(b"/repo/x").to_hex().to_string();
    let expected = root
        .join(".archon/workflows/write-locks")
        .join(format!("{hex}.lock"));
    assert_eq!(lock_path_for(root), expected);
}

#[test]
fn distinct_repos_distinct_lock_paths() {
    assert_ne!(
        lock_path_for(Path::new("/a")),
        lock_path_for(Path::new("/b"))
    );
}

#[test]
fn utf8_tail_no_invalid_bytes_on_boundary() {
    // 4095 'a' then a 3-byte char means the byte boundary splits the char.
    let mut bytes = vec![b'a'; 4095];
    bytes.extend_from_slice("€".as_bytes());
    let tail = utf8_safe_tail(&bytes, 4096);
    assert!(std::str::from_utf8(tail.as_bytes()).is_ok());
    assert!(tail.ends_with('€') || !tail.contains('\u{fffd}'));
}

#[test]
fn map_apply_error_carries_item() {
    let item = ItemId::from("impl-7");
    let err = map_apply_error_with_item(
        &item,
        IsolationError::ProcessFailed {
            stderr: "boom".into(),
        },
    );
    match err {
        ApplyError::PatchApplyConflict { item: i, stderr } => {
            assert_eq!(i, "impl-7");
            assert_eq!(stderr, "boom");
        }
        other => panic!("expected PatchApplyConflict, got {other:?}"),
    }
}

#[test]
fn from_isolation_never_patch_conflict() {
    let err: ApplyError = IsolationError::ProcessFailed { stderr: "x".into() }.into();
    assert!(!matches!(err, ApplyError::PatchApplyConflict { .. }));
}

#[test]
fn lock_blocks_then_succeeds() {
    let repo = canonical_repo();
    let root = repo.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let holder = thread::spawn({
        let root = root.clone();
        move || {
            with_repo_lock(&root, || {
                tx.send(()).unwrap();
                thread::sleep(Duration::from_millis(300));
                Ok(())
            })
            .unwrap();
        }
    });
    rx.recv().unwrap();
    let start = Instant::now();
    with_repo_lock(&root, || Ok(())).expect("eventually acquires");
    assert!(
        start.elapsed() >= Duration::from_millis(150),
        "should have waited for holder"
    );
    holder.join().unwrap();
}

#[test]
fn lock_times_out_when_held() {
    let repo = canonical_repo();
    let root = repo.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let holder = thread::spawn({
        let root = root.clone();
        move || {
            with_repo_lock(&root, || {
                tx.send(()).unwrap();
                done_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
        }
    });
    rx.recv().unwrap();
    let res: Result<(), ApplyError> = with_repo_lock_tuned(&root, 3, 30, || Ok(()));
    assert!(
        matches!(res, Err(ApplyError::LockTimeout { .. })),
        "got {res:?}"
    );
    done_tx.send(()).unwrap();
    holder.join().unwrap();
}
