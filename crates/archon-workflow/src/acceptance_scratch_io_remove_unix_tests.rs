//! #534 removal races on Unix: replaced entries, permission changes,
//! interrupted calls and substituting writers (seams in the parent module).
use super::*;

/// Review P1-2: a writer that swaps a directory for a link to a directory
/// outside the tree during the retry never redirects the removal there.
#[test]
fn owned_tree_removal_retry_never_follows_a_replacement_symlink() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = site();
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(outside.join("nested")).unwrap();
    std::fs::write(outside.join("victim"), b"keep").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o750)).unwrap();
    let mut added = false;
    let _listing = hook(move |path| {
        if path.ends_with("home") && !added {
            added = true;
            std::fs::write(path.join("leak"), b"x").unwrap();
        }
    });
    let target = outside.clone();
    let mut swapped = false;
    let _retry = hook_retry(move |path| {
        if path.ends_with("home") && !swapped {
            swapped = true;
            std::fs::rename(path, path.with_file_name("moved")).unwrap();
            std::os::unix::fs::symlink(&target, path).unwrap();
        }
    });
    let removal = remove_under_phase(30, &root);
    let mode = std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777;
    let kept = outside.join("victim").exists() && outside.join("nested").exists();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(kept, "nothing outside the owned tree is removed");
    assert_eq!(mode, 0o750, "nothing outside the owned tree is chmod-ed");
    removal.expect("the link and the moved directory are removed");
    assert!(!root.exists(), "removal ends only with the tree gone");
}

/// Owner access on a directory a check made unlistable is regained through
/// a handle that is that directory (no path is followed or chmod-ed).
#[test]
fn owned_tree_removal_regains_access_to_an_unreadable_directory() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, root) = site();
    std::fs::set_permissions(
        root.join("home/nested"),
        std::fs::Permissions::from_mode(0o300),
    )
    .unwrap();
    std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o100)).unwrap();
    remove_under_phase(30, &root).expect("an owned unlistable directory is still removed");
    assert!(!root.exists());
}

/// Review r2 P1: a writer that swaps an unlistable directory for a hard
/// link to a file outside the tree, after the listing open was refused and
/// before owner access is changed, never gets that file's mode changed.
#[test]
fn owned_tree_removal_permission_change_never_reaches_a_replacement_hard_link() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = site();
    let victim = temp.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o300)).unwrap();
    let outside = victim.clone();
    let mut swapped = false;
    let _chmod = hook_chmod(move |path| {
        if path.ends_with("home") && !swapped {
            swapped = true;
            std::fs::rename(path, path.with_file_name("moved")).unwrap();
            std::fs::hard_link(&outside, path).unwrap();
        }
    });
    let removal = remove_under_phase(30, &root);
    let mode = std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "nothing outside the owned tree is chmod-ed");
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    removal.expect("the link and the moved directory are removed");
    assert!(!root.exists(), "removal ends only with the tree gone");
}

/// Review r2 P2: a directory another remover took while its owner access
/// was being regained is removed, not a permission failure.
#[test]
fn owned_tree_removal_unlistable_directory_taken_before_chmod_is_removed() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, root) = site();
    std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o300)).unwrap();
    let mut taken = false;
    let _chmod = hook_chmod(move |path| {
        if path.ends_with("home") && !taken {
            taken = true;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::remove_dir_all(path).unwrap();
        }
    });
    remove_under_phase(30, &root).expect("an entry another remover took is removed");
    assert!(!root.exists());
}

/// Review r2 P2: interrupted calls (EINTR) are made again, including the
/// identity check of a directory a writer keeps non-empty for a while.
#[test]
fn owned_tree_removal_retries_interrupted_calls() {
    let (_temp, root) = site();
    let mut added = 0;
    let hooked = hook(move |path| {
        if path.ends_with("home") && added < 2 {
            added += 1;
            std::fs::write(path.join(format!("leak-{added}")), b"x").unwrap();
        }
    });
    INTERRUPTED.set(0);
    INTERRUPTING.set(true);
    let removal = remove_under_phase(30, &root);
    drop(hooked);
    removal.expect("an interrupted call is made again");
    assert!(INTERRUPTED.get() > 10, "calls were interrupted");
    assert!(!root.exists());
}

/// Review r2 P2: a failed `readdir` is an error, never the listing's end.
#[test]
fn owned_tree_removal_reports_a_failed_listing() {
    let (_temp, root) = site();
    let _hooked = hook(|_| {});
    LISTING_FAULT.set(Some(libc::EIO));
    let error = remove_under_phase(30, &root).expect_err("a failed listing is not the end");
    assert!(
        matches!(&error, WorkflowError::Io { source, .. } if source.raw_os_error() == Some(libc::EIO)),
        "{error:?}"
    );
}

/// Review r3 P1-A: a directory with no owner access at all (a check's
/// `chmod 000`) is removed on every Unix: where no handle can open it, it is
/// renamed to a fresh name, matched, and given owner access there.
#[test]
fn owned_tree_removal_regains_a_directory_without_owner_access() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = site();
    let nested = root.join("home/nested");
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o000)).unwrap();
    std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let removal = remove_under_phase(30, &root);
    if root.join("home").exists() {
        let _ = std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700));
    }
    removal.expect("an owned directory without owner access is removed");
    assert!(!root.exists(), "removal ends only with the tree gone");
    let left: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left.len(), 0, "nothing is left beside the tree: {left:?}");
}

/// Review r3 P1-A: a writer that swaps that directory for a hard link to a
/// file outside the tree just before the rename gets nothing chmod-ed: only
/// the renamed entry matched by device and inode gets owner access.
#[test]
fn owned_tree_removal_aside_rename_never_chmods_a_replacement_hard_link() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = site();
    let victim = temp.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let outside = victim.clone();
    let mut swapped = false;
    let _aside = hook_aside(move |path| {
        if path.ends_with("home") && !swapped {
            swapped = true;
            std::fs::rename(path, path.with_file_name("moved")).unwrap();
            std::fs::hard_link(&outside, path).unwrap();
        }
    });
    let removal = remove_under_phase(30, &root);
    let mode = std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777;
    for left in ["home", "moved"] {
        let _ = std::fs::set_permissions(root.join(left), std::fs::Permissions::from_mode(0o700));
    }
    assert_eq!(mode, 0o600, "nothing outside the owned tree is chmod-ed");
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    removal.expect("the link and the moved directory are removed");
    assert!(!root.exists(), "removal ends only with the tree gone");
}

/// Review r3 P1-B: a writer that keeps substituting a non-empty directory
/// for the one being removed ends as the phase's resumable stall: the
/// identity changes never renew the retry watch.
#[test]
fn owned_tree_removal_substituting_writer_is_a_resumable_stall() {
    let (_temp, root) = site();
    let substituted = std::rc::Rc::new(Cell::new(0u64));
    let count = substituted.clone();
    let _hook = hook(move |path| {
        // Bounded so a removal that never stalls still returns (red, not hung).
        if path.ends_with("home") && count.get() < 3000 {
            count.set(count.get() + 1);
            let aside = path.with_file_name(format!("aside-{}", count.get()));
            std::fs::rename(path, aside).unwrap();
            std::fs::create_dir(path).unwrap();
            std::fs::write(path.join("leak"), b"x").unwrap();
        }
    });
    let started = Instant::now();
    let error = remove_under_phase(1, &root).expect_err("a writer that never stops is no success");
    assert!(
        matches!(&error, WorkflowError::ControlPaused(why) if why.contains(control::OBSERVATION_STALLED)),
        "{error} after {} substitutions",
        substituted.get()
    );
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(
        substituted.get() < 3000,
        "the watch, not the writer, ended it"
    );
}

/// Review r4 P1-B: a writer that revokes owner access again after every
/// recovery rename ends as the phase's resumable stall: the same watch spans
/// every fresh name, and recovery continues in place (bounded stack).
/// macOS only: Linux regains access through an `O_PATH` handle and never
/// takes the rename-aside recovery path this test drives.
#[cfg(target_os = "macos")]
#[test]
fn owned_tree_removal_repeated_permission_revocation_is_a_resumable_stall() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, root) = site();
    std::fs::set_permissions(root.join("home"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let revoked = std::rc::Rc::new(Cell::new(0u64));
    let count = revoked.clone();
    let _hook = hook_after_aside(move |path| {
        // Bounded so a removal that never stalls still returns (red, not hung).
        if count.get() < 1000 {
            count.set(count.get() + 1);
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
    });
    let started = Instant::now();
    let removal = remove_under_phase(1, &root);
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
    }
    let error = removal.expect_err("a writer that never stops is no success");
    assert!(
        matches!(&error, WorkflowError::ControlPaused(why) if why.contains(control::OBSERVATION_STALLED)),
        "{error} after {} revocations",
        revoked.get()
    );
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(revoked.get() < 1000, "the watch, not the writer, ended it");
}
