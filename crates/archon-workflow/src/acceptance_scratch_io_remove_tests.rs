//! #534: an owned site's removal against a writer outside this process.
//! The seams run on the removing thread, at the exact points a racing
//! writer acts, so each race happens on every run.
// The listing seams fire only from the Unix handle-relative removal.
#![cfg_attr(not(unix), allow(dead_code))]
use super::*;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

type Hook = Box<dyn FnMut(&Path)>;
type Slot = std::thread::LocalKey<RefCell<Option<Hook>>>;
thread_local! {
    static AFTER_LISTING: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static BEFORE_RETRY: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static BEFORE_CHMOD: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static BEFORE_ASIDE: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static AFTER_ASIDE: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static WORKTREE: Cell<Git> = const { Cell::new(Git::Real) };
    static INTERRUPTING: Cell<bool> = const { Cell::new(false) };
    static INTERRUPTED: Cell<u64> = const { Cell::new(0) };
    static LISTING_FAULT: Cell<Option<i32>> = const { Cell::new(None) };
}

/// How the next `git worktree remove` of a site behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Git {
    Real,
    /// What real git leaves when a writer adds an entry while it deletes
    /// the copy: the worktree unregistered, the copy's directory kept with
    /// the writer's entry, and the failure.
    Race,
    /// Git fails before it unregisters the worktree: it stays registered.
    Refuse,
}

fn fire(slot: &'static Slot, path: &Path) {
    let hook = slot.with(|slot| slot.borrow_mut().take());
    if let Some(mut hook) = hook {
        hook(path);
        slot.with(|slot| {
            slot.borrow_mut().get_or_insert(hook);
        });
    }
}

/// Runs between a directory's listing and its removal.
pub(crate) fn after_listing(path: &Path) {
    fire(&AFTER_LISTING, path);
}

/// Runs after the wait, before a not-empty directory is listed again.
pub(crate) fn before_retry(path: &Path) {
    fire(&BEFORE_RETRY, path);
}

/// Runs after a directory refused to open for listing, before its owner
/// access is changed.
pub(crate) fn before_chmod(path: &Path) {
    fire(&BEFORE_CHMOD, path);
}

/// While a test interrupts calls, every other filesystem call of the
/// removal reports EINTR before it is made.
pub(crate) fn interrupt() -> bool {
    if !INTERRUPTING.get() {
        return false;
    }
    let calls = INTERRUPTED.get() + 1;
    INTERRUPTED.set(calls);
    calls % 2 == 1
}

/// A test's one failed `readdir` (null with this errno).
pub(crate) fn listing_fault() -> Option<i32> {
    LISTING_FAULT.take()
}

/// Runs after no handle could open a directory, before it is renamed aside.
pub(crate) fn before_aside(path: &Path) {
    fire(&BEFORE_ASIDE, path);
}

/// Runs after a directory got owner access under its fresh name, before it
/// is opened there.
pub(crate) fn after_aside(path: &Path) {
    fire(&AFTER_ASIDE, path);
}

/// The site's `git worktree remove`, as [`WORKTREE`] says.
pub(crate) fn worktree_remove(
    repository: &Path,
    remove: impl FnOnce() -> WorkflowResult<String>,
) -> WorkflowResult<String> {
    let failed = || {
        Err(invalid(format!(
            "scratch git command failed: error: failed to delete '{}': Directory not empty",
            repository.display()
        )))
    };
    match WORKTREE.replace(Git::Real) {
        Git::Real => remove(),
        Git::Race => {
            remove()?;
            std::fs::create_dir_all(repository).unwrap();
            std::fs::write(repository.join("leak"), b"x").unwrap();
            failed()
        }
        Git::Refuse => failed(),
    }
}

struct Hooked;
impl Drop for Hooked {
    fn drop(&mut self) {
        AFTER_LISTING.with(|slot| slot.borrow_mut().take());
        BEFORE_RETRY.with(|slot| slot.borrow_mut().take());
        BEFORE_CHMOD.with(|slot| slot.borrow_mut().take());
        BEFORE_ASIDE.with(|slot| slot.borrow_mut().take());
        AFTER_ASIDE.with(|slot| slot.borrow_mut().take());
        INTERRUPTING.set(false);
        LISTING_FAULT.set(None);
    }
}
fn hook(hook: impl FnMut(&Path) + 'static) -> Hooked {
    AFTER_LISTING.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    Hooked
}
fn hook_retry(hook: impl FnMut(&Path) + 'static) -> Hooked {
    BEFORE_RETRY.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    Hooked
}
fn hook_aside(hook: impl FnMut(&Path) + 'static) -> Hooked {
    BEFORE_ASIDE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    Hooked
}
// Only the macOS rename-aside regression drives this hook.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn hook_after_aside(hook: impl FnMut(&Path) + 'static) -> Hooked {
    AFTER_ASIDE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    Hooked
}
fn hook_chmod(hook: impl FnMut(&Path) + 'static) -> Hooked {
    BEFORE_CHMOD.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    Hooked
}

/// A site root with a `home` directory holding nested entries.
fn site() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("observation");
    std::fs::create_dir_all(root.join("home/nested")).unwrap();
    std::fs::write(root.join("home/held"), b"x").unwrap();
    std::fs::write(root.join("home/nested/file"), b"x").unwrap();
    (temp, root)
}

fn remove_under_phase(window: u64, root: &Path) -> WorkflowResult<()> {
    control::Control::new(window, Default::default()).run(|| remove_owned_tree(root))
}

#[test]
fn owned_tree_removal_reenumerates_entry_created_before_rmdir() {
    let (_temp, root) = site();
    let mut added = 0;
    let _hook = hook(move |path| {
        if path.ends_with("home") && added < 3 {
            added += 1;
            std::fs::write(path.join(format!("leak-{added}")), b"x").unwrap();
        }
    });
    remove_under_phase(30, &root).expect("a racing entry is listed again and removed");
    assert!(!root.exists(), "removal ends only with the tree gone");
}

#[test]
fn owned_tree_removal_tolerates_disappearing_entry() {
    let (_temp, root) = site();
    let mut taken = false;
    let _hook = hook(move |path| {
        if path.ends_with("home") && !taken {
            taken = true;
            std::fs::remove_dir_all(path).unwrap();
        }
    });
    remove_under_phase(30, &root).expect("an entry another remover took is removed");
    assert!(!root.exists());
}

#[test]
fn owned_tree_removal_persistent_writer_is_a_resumable_stall() {
    let (_temp, root) = site();
    let mut added = 0u64;
    let _hook = hook(move |path| {
        if path.ends_with("home") {
            added += 1;
            std::fs::write(path.join(format!("leak-{added}")), b"x").unwrap();
        }
    });
    let started = Instant::now();
    let error = remove_under_phase(1, &root).expect_err("a writer that never stops is no success");
    assert!(
        matches!(&error, WorkflowError::ControlPaused(why) if why.contains(control::OBSERVATION_STALLED)),
        "{error}"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "removing the writer's entries must not renew the wait"
    );
    assert!(
        root.join("home").exists(),
        "a stall never reports the tree gone"
    );
}

/// A committed repository and a policy for sites from it.
fn git_site() -> (tempfile::TempDir, ScratchPolicy, String) {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repository");
    let project = temp.path().join("workspace");
    let task_root = project.join("items");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(repository.join("input"), "data").unwrap();
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    ] {
        git(&repository, args);
    }
    let commit = git(&repository, &["rev-parse", "HEAD"]).trim().to_owned();
    let policy = ScratchPolicy {
        repository,
        project,
        task_root,
        scratch_parent: temp.path().join("scratch"),
        project_inputs: vec![],
        project_input_excludes: vec![],
        combined: false,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 4096,
        scratch_bytes: 16 * 1024 * 1024,
        build_cache: None,
    };
    (temp, policy, commit)
}

fn git(repository: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn teardown_takes_the_copy_git_left_after_a_racing_writer() {
    let (_temp, policy, commit) = git_site();
    let mut roots = ScratchRoots::prepare(&policy, &commit).unwrap();
    let root = roots.root().to_path_buf();
    let site = root.file_name().unwrap().to_string_lossy().into_owned();
    WORKTREE.set(Git::Race);
    roots
        .cleanup()
        .expect("the copy git left after a racing writer is removed as owned");
    assert_eq!(
        WORKTREE.get(),
        Git::Real,
        "teardown must reach the git removal"
    );
    assert!(!root.exists(), "teardown ends only with the site gone");
    assert!(
        !git(&policy.repository, &["worktree", "list", "--porcelain"]).contains(&site),
        "the site's worktree stays unregistered"
    );
}

/// Review P1-1: git lists the resolved path; a site reached through an
/// aliased root must still read as registered after a failed removal.
#[cfg(unix)]
#[test]
fn teardown_failed_removal_through_an_aliased_root_keeps_the_registered_site() {
    let (temp, policy, commit) = git_site();
    let mut roots = ScratchRoots::prepare(&policy, &commit).unwrap();
    let canonical = roots.root().to_path_buf();
    let site = canonical.file_name().unwrap().to_owned();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(canonical.parent().unwrap(), &alias).unwrap();
    roots.root = alias.join(&site);
    roots.repository = roots.root.join(SITE_REPOSITORY_DIRECTORY);
    let listed = || git(&policy.repository, &["worktree", "list", "--porcelain"]);
    let site = site.to_string_lossy().into_owned();
    WORKTREE.set(Git::Refuse);
    let error = roots
        .cleanup()
        .expect_err("a removal git refused is never reported done");
    assert!(error.to_string().contains("failed to delete"), "{error}");
    assert!(listed().contains(&site), "the site stays registered");
    assert!(
        canonical
            .join(SITE_REPOSITORY_DIRECTORY)
            .join("input")
            .exists(),
        "a registered worktree is never removed as an owned tree"
    );
    roots
        .cleanup()
        .expect("a later teardown removes the site through its alias");
    assert!(!canonical.exists() && !listed().contains(&site));
}

#[cfg(unix)]
#[path = "acceptance_scratch_io_remove_unix_tests.rs"]
mod unix_races;
