//! Batch E: what a worktree is seeded with, and what its capture keeps.
use super::*;
use crate::write_coordinator::project_inputs::write_test_policy;

fn git(root: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

struct World {
    dir: tempfile::TempDir,
    project: PathBuf,
    run_root: PathBuf,
    worktree: PathBuf,
}

/// A project with data under two inputs, and a worktree whose repository
/// ignores `.archon/*` but tracks one file under it.
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    write_test_policy(&run_root, &project, &[".archon/lab", "data"]);
    for (rel, body) in [
        (".archon/lab/data/registry.json", "registry-v1"),
        (".archon/lab/spec.json", "spec-project"),
        ("data/table.csv", "a,b"),
    ] {
        let path = project.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    let worktree = dir.path().join("worktree");
    std::fs::create_dir_all(worktree.join(".archon/lab")).unwrap();
    git(&worktree, &["init", "-q"]);
    std::fs::write(worktree.join(".gitignore"), ".archon/*\n").unwrap();
    std::fs::write(worktree.join(".archon/lab/spec.json"), "spec-repo").unwrap();
    git(&worktree, &["add", ".gitignore"]);
    git(&worktree, &["add", "-f", ".archon/lab/spec.json"]);
    git(
        &worktree,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "base",
        ],
    );
    World {
        dir,
        project,
        run_root,
        worktree,
    }
}

#[test]
fn only_what_git_ignores_and_does_not_track_is_seeded_and_a_share_is_made_private() {
    let w = world();
    // A host dependency share on the way: an ignored link into elsewhere.
    let shared = w.dir.path().join("canonical-share");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("registry.json"), "canonical").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&shared, w.worktree.join(".archon/lab/data")).unwrap();

    let seed = seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .expect("seeded");
    assert_eq!(seed.inputs, [".archon/lab", "data"]);
    // `.archon/lab` holds a tracked file, `data` is not ignored at all.
    assert!(seed.ignored_roots.is_empty(), "{seed:?}");
    assert_eq!(
        seed.files.keys().collect::<Vec<_>>(),
        [".archon/lab/data/registry.json"]
    );
    assert_eq!(
        seed.files[".archon/lab/data/registry.json"],
        hash(b"registry-v1")
    );
    let skipped: Vec<&str> = seed.skipped.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(skipped, [".archon/lab/spec.json", "data/table.csv"]);
    // The tracked file is git's; the share was replaced, never written through.
    assert_eq!(
        std::fs::read_to_string(w.worktree.join(".archon/lab/spec.json")).unwrap(),
        "spec-repo"
    );
    assert!(!w.worktree.join(".archon/lab/data").is_symlink());
    assert_eq!(
        std::fs::read_to_string(shared.join("registry.json")).unwrap(),
        "canonical"
    );
    assert_eq!(
        writable(&seed),
        [w.worktree.join(".archon/lab/data/registry.json")]
    );
    let told = preamble(&seed);
    assert!(
        told.contains(".archon/lab, data") && told.contains("data/table.csv"),
        "{told}"
    );
    // Nothing seeded is visible to git.
    let status = std::process::Command::new("git")
        .current_dir(&w.worktree)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(
        status.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[test]
fn a_capture_keeps_changed_new_and_deleted_inputs_against_their_seed() {
    let w = world();
    let seeded = seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .unwrap();
    std::fs::write(
        w.worktree.join(".archon/lab/data/registry.json"),
        "registry-v2",
    )
    .unwrap();
    std::fs::write(w.worktree.join(".archon/lab/data/new.json"), "new").unwrap();
    let tasks = vec!["TASK-1".to_string()];
    let changed = capture(&w.run_root, "impl", "a", &w.worktree, &tasks).unwrap();
    assert_eq!(
        changed,
        [
            ".archon/lab/data/new.json",
            ".archon/lab/data/registry.json"
        ]
    );
    let record: CaptureRecord = read_json(&capture_path(&w.run_root, "impl", "a")).unwrap();
    assert_eq!(record.task_ids, tasks);
    let registry = &record.changes[".archon/lab/data/registry.json"];
    assert_eq!(
        registry.baseline,
        seeded.files[".archon/lab/data/registry.json"]
    );
    assert_eq!(registry.post, hash(b"registry-v2"));
    assert_eq!(
        record.changes[".archon/lab/data/new.json"].baseline,
        "absent"
    );
    let kept = captured_bytes_dir(&w.run_root, "impl", "a").join(".archon/lab/data/new.json");
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "new");

    // A deletion is captured; an unchanged tree captures (and keeps) nothing.
    std::fs::remove_file(w.worktree.join(".archon/lab/data/registry.json")).unwrap();
    std::fs::remove_file(w.worktree.join(".archon/lab/data/new.json")).unwrap();
    let changed = capture(&w.run_root, "impl", "a", &w.worktree, &tasks).unwrap();
    assert_eq!(changed, [".archon/lab/data/registry.json"]);
    let record: CaptureRecord = read_json(&capture_path(&w.run_root, "impl", "a")).unwrap();
    assert_eq!(
        record.changes[".archon/lab/data/registry.json"].post,
        "deleted"
    );
    std::fs::write(
        w.worktree.join(".archon/lab/data/registry.json"),
        "registry-v1",
    )
    .unwrap();
    assert!(
        capture(&w.run_root, "impl", "a", &w.worktree, &tasks)
            .unwrap()
            .is_empty()
    );
    assert!(!capture_path(&w.run_root, "impl", "a").exists());
    // The project root itself was never touched by seeding or capture.
    assert_eq!(
        std::fs::read_to_string(w.project.join(".archon/lab/data/registry.json")).unwrap(),
        "registry-v1"
    );
}

#[test]
fn a_run_without_project_inputs_seeds_nothing() {
    let w = world();
    std::fs::remove_file(w.run_root.join("v2/generated-metadata.json")).unwrap();
    assert!(
        seed(&w.run_root, "impl", "a", &w.worktree)
            .unwrap()
            .is_none()
    );
    assert!(
        capture(&w.run_root, "impl", "a", &w.worktree, &[])
            .unwrap()
            .is_empty()
    );
}
