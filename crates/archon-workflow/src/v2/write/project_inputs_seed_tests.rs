//! Batch E: what a worktree is seeded with, and what its capture keeps.
use super::*;
use crate::write_coordinator::project_inputs::write_test_policy;
use archon_write_plan::ForbiddenPaths;

fn capture_all(w: &World, tasks: &[String]) -> InputCapture {
    capture(
        &w.run_root,
        ("impl", "a"),
        &w.worktree,
        tasks,
        &ForbiddenPaths::default(),
    )
    .unwrap()
}

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
    let told = super::super::project_inputs_report::preamble(&seed);
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
    let changed = capture_all(&w, &tasks).changed;
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
    let changed = capture_all(&w, &tasks).changed;
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
    assert!(capture_all(&w, &tasks).changed.is_empty());
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
    assert!(capture_all(&w, &[]).changed.is_empty());
}

#[test]
fn seeding_again_clears_what_ran_since_and_a_forbidden_or_odd_change_is_left_out() {
    let w = world();
    seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .unwrap();
    // A baseline command wrote under the inputs; the next seed clears it.
    std::fs::write(w.worktree.join(".archon/lab/data/scratch.tmp"), "junk").unwrap();
    std::fs::write(w.worktree.join(".archon/lab/data/registry.json"), "mutated").unwrap();
    seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .unwrap();
    assert!(!w.worktree.join(".archon/lab/data/scratch.tmp").exists());
    assert_eq!(
        std::fs::read_to_string(w.worktree.join(".archon/lab/data/registry.json")).unwrap(),
        "registry-v1"
    );
    assert!(capture_all(&w, &[]).changed.is_empty());

    // A forbidden path and a link are reported, never kept.
    std::fs::write(w.worktree.join(".archon/lab/data/registry.json"), "v2").unwrap();
    std::fs::write(w.worktree.join(".archon/lab/data/secret.json"), "s").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hosts", w.worktree.join(".archon/lab/data/hosts")).unwrap();
    let forbidden = ForbiddenPaths::from_entries(["`.archon/lab/data/secret.json`"]);
    let got = capture(&w.run_root, ("impl", "a"), &w.worktree, &[], &forbidden).unwrap();
    assert_eq!(got.changed, [".archon/lab/data/registry.json"]);
    let dropped: Vec<&str> = got.dropped.iter().map(|(p, _)| p.as_str()).collect();
    #[cfg(unix)]
    assert_eq!(
        dropped,
        [".archon/lab/data/hosts", ".archon/lab/data/secret.json"]
    );
    #[cfg(not(unix))]
    assert_eq!(dropped, [".archon/lab/data/secret.json"]);
    let mut result = crate::v2::WorkflowV2Result::accepted("done");
    super::super::project_inputs_report::report_capture(&mut result, "impl-0", &got);
    assert_eq!(result.residual_gaps[0].severity.as_deref(), Some("high"));
    assert!(result.residual_gaps[0].description.contains("secret.json"));
}

#[test]
fn a_file_too_large_to_seed_keeps_its_baseline_so_a_regenerated_copy_can_land() {
    let w = world();
    write_test_policy(&w.run_root, &w.project, &[".archon/lab"]);
    let big = w.project.join(".archon/lab/data/big.bin");
    std::fs::write(&big, vec![7u8; 64]).unwrap();
    // The policy's own scratch cap bounds the seed: here, a few bytes.
    let metadata = w.run_root.join("v2/generated-metadata.json");
    let text = std::fs::read_to_string(&metadata).unwrap();
    std::fs::write(&metadata, text.replace("1073741824", "20")).unwrap();
    let seeded = seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .unwrap();
    assert!(!seeded.files.contains_key(".archon/lab/data/big.bin"));
    // Recorded by size and time, never read.
    let baseline = seeded.baseline(".archon/lab/data/big.bin");
    assert!(baseline.starts_with("meta:64:"), "{baseline}");
    std::fs::write(w.worktree.join(".archon/lab/data/big.bin"), "small").unwrap();
    let got = capture_all(&w, &[]);
    let record: CaptureRecord = read_json(&capture_path(&w.run_root, "impl", "a")).unwrap();
    assert!(
        got.changed
            .contains(&".archon/lab/data/big.bin".to_string())
    );
    assert_eq!(
        record.changes[".archon/lab/data/big.bin"].baseline,
        baseline
    );
    // Over the cap at capture: nothing is kept and the patch is untouched.
    std::fs::write(w.worktree.join(".archon/lab/data/big.bin"), vec![1u8; 64]).unwrap();
    let got = capture_all(&w, &[]);
    assert!(got.changed.is_empty() && got.refused.is_some(), "{got:?}");
    assert!(!capture_path(&w.run_root, "impl", "a").exists());
}

/// Batch I2: a project-input file named like the run's frozen acceptance
/// contract but not byte-identical to it is never seeded (an agent would
/// take it for the contract the harness runs); it is recorded as excluded
/// and told to the agent. An identical copy is seeded as before.
#[test]
fn a_stale_copy_of_the_frozen_contract_is_never_seeded() {
    let w = world();
    let tasks = w.project.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("acceptance-contract.json"), "frozen").unwrap();
    std::fs::write(
        w.project.join(".archon/lab/acceptance-contract.json"),
        "draft",
    )
    .unwrap();
    std::fs::write(
        w.project.join(".archon/lab/data/acceptance-contract.json"),
        "frozen",
    )
    .unwrap();
    let seeded = seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .unwrap();
    assert!(
        !w.worktree
            .join(".archon/lab/acceptance-contract.json")
            .exists()
    );
    assert!(
        !seeded
            .files
            .contains_key(".archon/lab/acceptance-contract.json")
    );
    let (_, why) = (seeded.skipped.iter())
        .find(|(path, _)| path == ".archon/lab/acceptance-contract.json")
        .unwrap_or_else(|| panic!("{seeded:?}"));
    assert!(
        why.starts_with("excluded: it shadows the run's frozen acceptance contract"),
        "{why}"
    );
    assert!(
        why.contains(&tasks.join("acceptance-contract.json").display().to_string()),
        "{why}"
    );
    let recorded: SeedRecord = read_json(&seed_path(&w.run_root, "impl", "a")).unwrap();
    assert_eq!(recorded.skipped, seeded.skipped);
    assert!(
        super::super::project_inputs_report::preamble(&seeded).contains("excluded: it shadows")
    );
    // The project's copy is project data: untouched.
    assert_eq!(
        std::fs::read(w.project.join(".archon/lab/acceptance-contract.json")).unwrap(),
        b"draft"
    );
    // A write at the excluded path never lands; nor does a case variant seed.
    std::fs::write(
        w.worktree.join(".archon/lab/acceptance-contract.json"),
        "agent",
    )
    .unwrap();
    let got = capture_all(&w, &[]);
    assert!(
        !got.changed
            .contains(&".archon/lab/acceptance-contract.json".to_string()),
        "{got:?}"
    );
    assert!(
        got.dropped
            .iter()
            .any(|(p, _)| p == ".archon/lab/acceptance-contract.json"),
        "{got:?}"
    );
    std::fs::remove_file(w.project.join(".archon/lab/acceptance-contract.json")).unwrap();
    std::fs::write(
        w.project.join(".archon/lab/Acceptance-Contract.JSON"),
        "draft",
    )
    .unwrap();
    let again = seed(&w.run_root, "impl", "a", &w.worktree)
        .unwrap()
        .unwrap();
    assert!(
        !again
            .files
            .contains_key(".archon/lab/Acceptance-Contract.JSON"),
        "{again:?}"
    );
    let seeded = again;
    // An identical copy shadows nothing, but no change to it ever lands.
    std::fs::write(
        w.worktree.join(".archon/lab/data/acceptance-contract.json"),
        "edited",
    )
    .unwrap();
    assert!(capture_all(&w, &[]).changed.is_empty());
    assert!(
        seeded
            .files
            .contains_key(".archon/lab/data/acceptance-contract.json"),
        "{seeded:?}"
    );
}
