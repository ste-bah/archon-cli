//! Issue-266: an explicit restart revokes every stored outcome of the
//! restarted branches, the current one and every superseded one, from reuse
//! and from the landed-task set. A superseded `patch_landed` record is never
//! restored as the landing record afterwards. The history stays on disk for
//! audit. Driven through the real restart entry points and the real branch
//! cache; every check reads the files back.

#[path = "support/restart_run.rs"]
mod restart_run;

use archon_workflow::v2::branch_cache::split_reusable_branch_outcomes;
use archon_workflow::v2::restart::{invalidate_generated_v2_item, restart_generated_v2_task};
use archon_workflow::v2::reuse_identity::reuse_identity;
use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
    WorkflowV2WriteMode,
};
use restart_run::{generated_run, v2_store};

const CALL: &str = "implementation-wave-1";

/// The write branch of `task` in the wave, as the host derives it.
fn item(task: &str) -> WorkflowV2FanoutItem {
    let id = format!("{CALL}-{task}");
    let call = WorkflowV2HostCall {
        id: id.clone(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    WorkflowV2FanoutItem::read_only(
        id,
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": CALL,
            "fanout_item_id": task,
            "item": { "id": task, "canonical_task_ids": [task], "target_files": ["src/lib.rs"] },
        }),
    )
}

/// An outcome of `task`'s branch at `status`, recorded with `hash`.
fn outcome(
    task: &str,
    status: WorkflowV2Status,
    hash: &str,
    landed: bool,
) -> WorkflowV2BranchOutcome {
    let branch = item(task);
    let mut result = WorkflowV2Result::accepted("branch produced the declared change");
    result.status = status;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "branch recorded concrete implementation evidence",
    ));
    result.data = serde_json::json!({
        "branch_id": branch.id,
        "canonical_task_ids": [task],
        "patch_landed": landed,
    });
    WorkflowV2BranchOutcome {
        item_id: branch.id.clone(),
        role: "coder".to_string(),
        status,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: Some(hash.to_string()),
        completion_evidence: vec![WorkflowV2TaskCompletionEvidence::new(
            task,
            WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate,
            CALL,
            &branch.id,
            status,
        )],
    }
}

/// `task` landed (accepted, `patch_landed`, hash H1), then a later save with
/// another hash (a replay's no-op, H2) superseded that record.
fn landed_then_superseded(v2: &WorkflowV2ResultStore, task: &str) {
    v2.save_branch_outcome(CALL, &outcome(task, WorkflowV2Status::Accepted, "H1", true))
        .unwrap();
    v2.save_branch_outcome(CALL, &outcome(task, WorkflowV2Status::Noop, "H2", false))
        .unwrap();
    let archive = v2.branch_outcome_path(CALL, &item(task).id);
    let archive = archive.parent().unwrap().join("superseded");
    assert_eq!(
        std::fs::read_dir(archive).unwrap().count(),
        1,
        "H1 archived"
    );
}

/// `(reused, pending)` item ids for one split of `tasks`.
fn split(v2: &WorkflowV2ResultStore, tasks: &[&str]) -> (Vec<String>, Vec<String>) {
    let items = tasks.iter().map(|task| item(task)).collect();
    let (reused, pending) = split_reusable_branch_outcomes(v2, CALL, items).unwrap();
    (
        reused.into_iter().map(|outcome| outcome.item_id).collect(),
        pending.into_iter().map(|item| item.id).collect(),
    )
}

fn assert_revoked(v2: &WorkflowV2ResultStore, task: &str) {
    let branch = item(task).id;
    let (reused, pending) = split(v2, &[task]);
    assert!(
        reused.is_empty(),
        "{task}: revoked outcome reused: {reused:?}"
    );
    assert_eq!(pending, vec![branch.clone()]);
    assert!(
        !v2.branch_outcome_path(CALL, &branch).exists(),
        "{task}: the branch cache wrote a revoked outcome back into the slot"
    );
}

#[test]
fn restart_task_revokes_a_superseded_landing_record() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    assert_eq!(split(&v2, &["T-A"]).0.len(), 1, "landed before the restart");

    restart_generated_v2_task(&store, &run, "T-A")
        .unwrap()
        .expect("generated run");

    assert_revoked(&v2, "T-A");
    assert!(
        v2.load_superseded_branch_outcomes().is_empty(),
        "the superseded landing record still feeds the landed-task set"
    );
}

#[test]
fn restart_agent_item_revokes_a_superseded_landing_record() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");

    invalidate_generated_v2_item(&store, &run, CALL, "T-A").unwrap();

    assert_revoked(&v2, "T-A");
}

#[test]
fn a_restart_keeps_the_revoked_history_and_the_other_tasks_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let other = item("T-C");
    let mut kept = outcome("T-C", WorkflowV2Status::Accepted, "", false);
    kept.item_input_hash = Some(reuse_identity(&other));
    v2.save_branch_outcome(CALL, &kept).unwrap();

    restart_generated_v2_task(&store, &run, "T-A").unwrap();

    // The unrelated task is still reused; the restarted one runs again.
    let (reused, pending) = split(&v2, &["T-A", "T-C"]);
    assert_eq!(reused, vec![other.id.clone()]);
    assert_eq!(pending, vec![item("T-A").id]);
    // Both revoked records of T-A are kept on disk for audit.
    let dir = v2.branch_outcome_path(CALL, &other.id);
    let revoked = std::fs::read_dir(dir.parent().unwrap().join("revoked"))
        .expect("revoked history")
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path()).ok())
        .filter_map(|raw| serde_json::from_slice::<WorkflowV2BranchOutcome>(&raw).ok())
        .map(|outcome| outcome.item_input_hash.unwrap_or_default())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        revoked,
        ["H1".to_string(), "H2".to_string()].into_iter().collect()
    );
}

/// Round 2: the landed-task set reads `result.data.canonical_task_ids`, so
/// the revocation selects by it too. An outcome whose completion evidence is
/// empty (a call id that mints none) is still revoked.
#[test]
fn restart_task_revokes_an_outcome_known_only_by_its_canonical_task_ids() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    for (status, hash, landed) in [
        (WorkflowV2Status::Accepted, "H1", true),
        (WorkflowV2Status::Noop, "H2", false),
    ] {
        let mut saved = outcome("T-A", status, hash, landed);
        saved.completion_evidence.clear();
        v2.save_branch_outcome(CALL, &saved).unwrap();
    }

    restart_generated_v2_task(&store, &run, "T-A").unwrap();

    assert_revoked(&v2, "T-A");
}

/// Round 2: an archived outcome that cannot be read is not "nothing to
/// revoke". The restart fails, names the file, and revokes nothing.
#[cfg(unix)]
#[test]
fn an_unreadable_archived_outcome_fails_the_restart_and_revokes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    let archived = std::fs::read_dir(current.parent().unwrap().join("superseded"))
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    let mode = |bits| std::fs::Permissions::from_mode(bits);
    std::fs::set_permissions(&archived, mode(0o000)).unwrap();

    let result = restart_generated_v2_task(&store, &run, "T-A");

    std::fs::set_permissions(&archived, mode(0o644)).unwrap();
    let error = result.expect_err("an unreadable archive must fail the restart");
    let name = archived.file_name().unwrap().to_string_lossy().into_owned();
    assert!(error.to_string().contains(&name), "{error}");
    assert!(current.exists(), "the current outcome was revoked anyway");
    assert!(!current.parent().unwrap().join("revoked").exists());
}

#[cfg(unix)]
#[test]
fn restart_revokes_a_superseded_symlink_that_the_cache_can_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    let target = temp.path().join("landing.json");
    std::fs::write(
        &target,
        serde_json::to_vec(&outcome("T-A", WorkflowV2Status::Accepted, "H1", true)).unwrap(),
    )
    .unwrap();
    let archive = v2
        .branch_outcome_path(CALL, &item("T-A").id)
        .parent()
        .unwrap()
        .join("superseded");
    std::fs::create_dir_all(&archive).unwrap();
    let link = archive.join("landing.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(v2.load_superseded_branch_outcomes().len(), 1);
    assert_eq!(split(&v2, &["T-A"]).0.len(), 1);
    restart_generated_v2_task(&store, &run, "T-A").unwrap();
    assert!(
        !link.exists(),
        "restart left a reusable symlink in the archive"
    );
    assert_revoked(&v2, "T-A");
    assert!(v2.load_superseded_branch_outcomes().is_empty());
    assert!(
        target.exists(),
        "revocation moves the link, not its external target"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_superseded_symlink_aborts_revocation_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    let before = std::fs::read(&current).unwrap();
    let link = current.parent().unwrap().join("superseded/broken.json");
    std::os::unix::fs::symlink(temp.path().join("missing"), &link).unwrap();
    let restarted = restart_generated_v2_task(&store, &run, "T-A");
    assert!(restarted.is_err(), "{restarted:?}");
    assert_eq!(std::fs::read(current).unwrap(), before);
    assert_eq!(v2.restart_epoch().unwrap(), 0);
}

#[test]
fn restart_revokes_a_superseded_non_json_landing_record() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let archive = v2
        .branch_outcome_path(CALL, &item("T-A").id)
        .parent()
        .unwrap()
        .join("superseded");
    let old = std::fs::read_dir(&archive)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let renamed = archive.join("landing.record");
    std::fs::rename(old, &renamed).unwrap();
    assert_eq!(
        split(&v2, &["T-A"]).0.len(),
        1,
        "the landing reader accepts this filename"
    );
    restart_generated_v2_task(&store, &run, "T-A").unwrap();
    assert!(
        !renamed.exists(),
        "restart left a landing record reachable by reuse"
    );
    assert_revoked(&v2, "T-A");
}

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
    let cases: [(&str, fn(&std::path::Path, &std::path::Path)); 3] = [
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
