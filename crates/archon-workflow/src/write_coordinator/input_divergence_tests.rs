//! Batch G (C, D): a tracked input whose project copy diverged makes the
//! combined scratch refuse to build, naming both sources; the host restores
//! the tracked copy (keeping the diverged one) and the scratch then builds.
//! A copy a recorded landing put there is left alone.
use std::path::{Path, PathBuf};

use super::*;
use crate::acceptance_scratch::{ScratchPolicy, ScratchRoots};

struct Layout {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    project: PathBuf,
    run_root: PathBuf,
    policy: ScratchPolicy,
    commit: String,
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

const TRACKED: &str = "{\"datasets\":[\"a\"]}";
const DIVERGED: &str = "{\"datasets\":[\"a\",\"regenerated\"]}";

fn layout() -> Layout {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let project = base.join("project");
    let spec = "data/strategies/s1/strategy-spec.json";
    std::fs::create_dir_all(repo.join("data/strategies/s1")).unwrap();
    std::fs::write(repo.join(spec), TRACKED).unwrap();
    std::fs::write(repo.join("lib.txt"), "code").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::create_dir_all(project.join("data/strategies/s1")).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::write(project.join(spec), DIVERGED).unwrap();
    std::fs::write(project.join("data/untracked.csv"), "x,y").unwrap();
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    let policy = ScratchPolicy {
        repository: repo.clone(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: base.join("scratch"),
        project_inputs: vec![PathBuf::from("data")],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: std::env::join_paths([base.join("tools")])
            .unwrap()
            .into_string()
            .unwrap(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 2048,
        scratch_bytes: 16_777_216,
        build_cache: None,
    };
    policy.validate().expect("valid native fixture policy");
    Layout {
        _dir: dir,
        repo,
        project,
        run_root,
        policy,
        commit,
    }
}

#[test]
fn a_diverged_tracked_input_is_restored_and_the_scratch_then_builds() {
    let layout = layout();
    let spec = layout.project.join("data/strategies/s1/strategy-spec.json");
    // D: the refusal names the project's copy and the repository's commit.
    let error = ScratchRoots::prepare(&layout.policy, &layout.commit)
        .err()
        .expect("the diverged copy collides");
    let text = error.to_string();
    assert!(
        text.contains("nonidentical scratch path collision")
            && text.contains(&layout.project.display().to_string())
            && text.contains(&layout.commit)
            && text.contains("tracked copy"),
        "{text}"
    );

    let divergences =
        restore_diverged_tracked_inputs(&layout.run_root, &layout.policy, &layout.commit).unwrap();
    assert_eq!(divergences.len(), 1, "{divergences:?}");
    let restored = &divergences[0];
    assert!(restored.restored, "{restored:?}");
    assert_eq!(restored.path, "data/strategies/s1/strategy-spec.json");
    assert_eq!(std::fs::read_to_string(&spec).unwrap(), TRACKED);
    let kept = restored.kept_at.as_ref().unwrap();
    assert_eq!(std::fs::read_to_string(kept).unwrap(), DIVERGED);
    assert!(restored.describe(&layout.commit).contains("host restored"));
    assert_eq!(
        std::fs::read_to_string(layout.project.join("data/untracked.csv")).unwrap(),
        "x,y",
        "untracked project data is the project's and is never touched"
    );
    let log = std::fs::read_to_string(
        layout
            .run_root
            .join("write-coordination/project-inputs-restored.jsonl"),
    )
    .unwrap();
    assert!(log.contains("strategy-spec.json"), "{log}");

    // Native scratch target links are Unix-only; the collision and restore
    // above remain checked on every host.
    #[cfg(unix)]
    {
        let mut roots =
            ScratchRoots::prepare(&layout.policy, &layout.commit).expect("now it builds");
        assert_eq!(
            std::fs::read_to_string(
                roots
                    .project()
                    .join("data/strategies/s1/strategy-spec.json")
            )
            .unwrap(),
            TRACKED
        );
        roots.cleanup().unwrap();
    }
    // In step: nothing more to do.
    assert!(
        restore_diverged_tracked_inputs(&layout.run_root, &layout.policy, &layout.commit)
            .unwrap()
            .is_empty()
    );
    let _ = &layout.repo;
}

/// A copy a landing this run recorded placing is not the host's to undo.
#[test]
fn a_copy_a_recorded_landing_put_there_is_left_and_named() {
    let layout = layout();
    let spec = layout.project.join("data/strategies/s1/strategy-spec.json");
    let state = blake3::hash(DIVERGED.as_bytes()).to_hex().to_string();
    let ledger = layout
        .run_root
        .join("write-coordination/project-inputs.jsonl");
    std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    std::fs::write(
        &ledger,
        format!(
            "{}\n",
            serde_json::json!({"stage_id":"s","item_id":"i","task_ids":["T"],
                "path":"data/strategies/s1/strategy-spec.json","outcome":"applied",
                "before":"x","after":state,"at":1})
        ),
    )
    .unwrap();
    let divergences =
        restore_diverged_tracked_inputs(&layout.run_root, &layout.policy, &layout.commit).unwrap();
    assert_eq!(divergences.len(), 1);
    assert!(!divergences[0].restored);
    assert!(
        divergences[0].reason.contains("landing s/i"),
        "{divergences:?}"
    );
    assert_eq!(std::fs::read_to_string(&spec).unwrap(), DIVERGED);
}

/// Outside the combined view no collision is possible: nothing is touched.
#[test]
fn a_separate_project_view_is_left_alone() {
    let mut layout = layout();
    layout.policy.combined = false;
    assert!(
        restore_diverged_tracked_inputs(&layout.run_root, &layout.policy, &layout.commit)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(layout.project.join("data/strategies/s1/strategy-spec.json"))
            .unwrap(),
        DIVERGED
    );
}

/// Review M4: a write call's declared delivery the tripwire recorded is
/// explained, and left alone.
#[test]
fn a_recorded_delivery_is_left_and_named() {
    let layout = layout();
    let spec = layout.project.join("data/strategies/s1/strategy-spec.json");
    let state = blake3::hash(DIVERGED.as_bytes()).to_hex().to_string();
    let ledger = layout
        .run_root
        .join("write-coordination/project-inputs-delivered.jsonl");
    std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    std::fs::write(
        &ledger,
        format!(
            "{}\n",
            serde_json::json!({"call":"w","path":"data/strategies/s1/strategy-spec.json","after":state})
        ),
    )
    .unwrap();
    let divergences =
        restore_diverged_tracked_inputs(&layout.run_root, &layout.policy, &layout.commit).unwrap();
    assert!(!divergences[0].restored, "{divergences:?}");
    assert_eq!(std::fs::read_to_string(&spec).unwrap(), DIVERGED);
}
