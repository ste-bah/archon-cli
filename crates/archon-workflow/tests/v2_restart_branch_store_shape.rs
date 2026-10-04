//! Issue-266: restart refuses a branch store that is not made of real
//! directories, and never blocks on or reuses special files planted in it.

#[path = "support/branch_revocation.rs"]
mod branch_revocation;
#[path = "support/restart_run.rs"]
mod restart_run;

use archon_workflow::v2::restart::{invalidate_generated_v2_item, restart_generated_v2_task};
use branch_revocation::*;
use restart_run::{generated_run, v2_store};

#[cfg(unix)]
#[test]
fn a_fifo_in_the_archive_never_blocks_restart() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let fifo = v2
        .branch_outcome_path(CALL, &item("T-A").id)
        .parent()
        .unwrap()
        .join("superseded/planted.json");
    let fifo_path = fifo.clone();
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ =
            done.send(restart_generated_v2_task(&store, &run, "T-A").map_err(|e| e.to_string()));
    });
    let restarted = wait
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("restart blocked on a FIFO in the archive");
    restarted.unwrap();
    // Quarantined unread: no reader can take outcome bytes from it later.
    assert!(
        std::fs::symlink_metadata(&fifo_path).is_err(),
        "the FIFO stayed in the reusable archive"
    );
    let revoked = fifo_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("revoked");
    assert!(std::fs::read_dir(revoked).unwrap().any(|entry| {
        let name = entry.unwrap().file_name();
        name.to_string_lossy().starts_with("planted-")
    }));
    assert_revoked(&v2, "T-A");
}

#[cfg(unix)]
#[test]
fn a_fifo_current_slot_is_quarantined_once_with_its_branch_history() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    std::fs::remove_file(&current).unwrap();
    let path = std::ffi::CString::new(current.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(
            restart_generated_v2_task(&store, &run, "T-A").map_err(|error| error.to_string()),
        );
    });
    wait.recv_timeout(std::time::Duration::from_secs(20))
        .expect("restart blocked on a FIFO current slot")
        .unwrap();
    assert!(std::fs::symlink_metadata(&current).is_err());
    assert_revoked(&v2, "T-A");
}

#[cfg(unix)]
#[test]
fn a_linked_superseded_archive_is_refused_before_any_move() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    let call_dir = current.parent().unwrap().to_path_buf();
    let archive = call_dir.join("superseded");
    std::fs::rename(&archive, call_dir.join("moved-away")).unwrap();
    std::os::unix::fs::symlink(".", &archive).unwrap();
    let before = std::fs::read(&current).unwrap();
    let restarted = restart_generated_v2_task(&store, &run, "T-A");
    let error = restarted.expect_err("a linked archive must be refused");
    assert!(error.to_string().contains("superseded"), "{error}");
    assert_eq!(std::fs::read(&current).unwrap(), before);
    assert!(!call_dir.join("revoked").exists());
    assert_eq!(v2.restart_epoch().unwrap(), 0);
}

/// Each directory level of the branch store aliased by a link: refused
/// before any move, for task restart and for item revocation.
#[cfg(unix)]
#[test]
fn every_linked_branch_store_level_is_refused_before_any_move() {
    type Alias = fn(&std::path::Path, &std::path::Path);
    let cases: [(&str, Alias); 3] = [
        ("call", |branches, call| {
            let away = branches.parent().unwrap().join("call-away");
            std::fs::rename(call, &away).unwrap();
            std::os::unix::fs::symlink(&away, call).unwrap();
        }),
        ("branches", |branches, _| {
            let away = branches.parent().unwrap().join("branches-away");
            std::fs::rename(branches, &away).unwrap();
            std::os::unix::fs::symlink(&away, branches).unwrap();
        }),
        ("revoked", |_, call| {
            std::os::unix::fs::symlink("superseded", call.join("revoked")).unwrap();
        }),
    ];
    for (name, alias) in cases {
        let temp = tempfile::tempdir().unwrap();
        let (store, run) = generated_run(&temp, &[CALL]);
        let v2 = v2_store(&store, &run);
        landed_then_superseded(&v2, "T-A");
        let current = v2.branch_outcome_path(CALL, &item("T-A").id);
        let call = current.parent().unwrap().to_path_buf();
        alias(call.parent().unwrap(), &call);
        let before = std::fs::read(&current).unwrap();
        let superseded = std::fs::read_dir(call.join("superseded")).unwrap().count();
        assert!(
            restart_generated_v2_task(&store, &run, "T-A").is_err(),
            "{name}"
        );
        assert!(
            v2.revoke_branch_outcome(CALL, &item("T-A").id).is_err(),
            "{name}"
        );
        assert_eq!(std::fs::read(&current).unwrap(), before, "{name}");
        let after = std::fs::read_dir(call.join("superseded")).unwrap().count();
        assert_eq!(after, superseded, "{name}: the archive changed");
        assert_eq!(v2.restart_epoch().unwrap(), 0, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn item_revocation_refuses_a_linked_archive_before_any_move() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    let call = current.parent().unwrap().to_path_buf();
    std::fs::rename(call.join("superseded"), call.join("moved-away")).unwrap();
    std::os::unix::fs::symlink(".", call.join("superseded")).unwrap();
    let before = std::fs::read(&current).unwrap();
    assert!(v2.revoke_branch_outcome(CALL, &item("T-A").id).is_err());
    assert_eq!(std::fs::read(&current).unwrap(), before);
    assert!(!call.join("revoked").exists());
    let _ = store;
}

#[cfg(unix)]
#[test]
fn item_restart_and_a_file_at_revoked_are_refused_before_any_mutation() {
    for case in [
        "item-restart-linked-call",
        "task-restart-file-at-revoked",
        "item-restart-file-call",
        "item-restart-broken-link",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (store, run) = generated_run(&temp, &[CALL]);
        let v2 = v2_store(&store, &run);
        landed_then_superseded(&v2, "T-A");
        let current = v2.branch_outcome_path(CALL, &item("T-A").id);
        let call = current.parent().unwrap().to_path_buf();
        let state_before = std::fs::read(store.run_dir(&run.id).join("state.json")).unwrap();
        let refused = if case == "item-restart-linked-call" {
            let away = call.parent().unwrap().parent().unwrap().join("call-away");
            std::fs::rename(&call, &away).unwrap();
            std::os::unix::fs::symlink(&away, &call).unwrap();
            invalidate_generated_v2_item(&store, &run, CALL, "T-A").is_err()
        } else if case == "item-restart-file-call" {
            std::fs::remove_dir_all(&call).unwrap();
            std::fs::write(&call, b"not a directory").unwrap();
            invalidate_generated_v2_item(&store, &run, CALL, "T-A").is_err()
        } else if case == "item-restart-broken-link" {
            let link = call.join("superseded/broken.json");
            std::os::unix::fs::symlink(call.join("missing"), link).unwrap();
            invalidate_generated_v2_item(&store, &run, CALL, "T-A").is_err()
        } else {
            std::fs::write(call.join("revoked"), b"not a directory").unwrap();
            restart_generated_v2_task(&store, &run, "T-A").is_err()
        };
        assert!(refused, "{case}");
        let state_after = std::fs::read(store.run_dir(&run.id).join("state.json")).unwrap();
        assert_eq!(state_before, state_after, "{case}: state changed");
        assert_eq!(v2.restart_epoch().unwrap(), 0, "{case}");
        assert!(
            case == "item-restart-file-call" || current.exists(),
            "{case}"
        );
    }
}
