//! Issue-266: an explicit restart revokes every stored outcome of the
//! restarted branches, the current one and every superseded one, from reuse
//! and from the landed-task set. A superseded `patch_landed` record is never
//! restored as the landing record afterwards. The history stays on disk for
//! audit. Driven through the real restart entry points and the real branch
//! cache; every check reads the files back.

#[path = "support/restart_run.rs"]
mod restart_run;

use archon_workflow::v2::restart::{invalidate_generated_v2_item, restart_generated_v2_task};
use archon_workflow::v2::reuse_identity::reuse_identity;
use archon_workflow::{WorkflowV2BranchOutcome, WorkflowV2Status};
use restart_run::{generated_run, v2_store};
#[path = "support/branch_revocation.rs"]
mod branch_revocation;

use branch_revocation::*;

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

/// A link the cache can reuse is one that resolves inside the archive
/// (Issue-292: a link out of it is never read). Restart moves the link and
/// the record it names, so neither stays reusable.
#[cfg(unix)]
#[test]
fn restart_revokes_a_superseded_symlink_that_the_cache_can_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    let archive = v2
        .branch_outcome_path(CALL, &item("T-A").id)
        .parent()
        .unwrap()
        .join("superseded");
    std::fs::create_dir_all(&archive).unwrap();
    let target = archive.join("landing.record");
    std::fs::write(
        &target,
        serde_json::to_vec(&outcome("T-A", WorkflowV2Status::Accepted, "H1", true)).unwrap(),
    )
    .unwrap();
    let link = archive.join("landing.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(v2.load_superseded_branch_outcomes().len(), 1);
    assert_eq!(split(&v2, &["T-A"]).0.len(), 1);
    restart_generated_v2_task(&store, &run, "T-A").unwrap();
    assert!(
        std::fs::symlink_metadata(&link).is_err(),
        "restart left a reusable symlink in the archive"
    );
    assert!(
        !target.exists(),
        "restart left the linked record in the archive"
    );
    assert_revoked(&v2, "T-A");
    assert!(v2.load_superseded_branch_outcomes().is_empty());
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
