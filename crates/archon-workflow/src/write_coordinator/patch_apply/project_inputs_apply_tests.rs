//! Batch E: a landing's project-input changes reach the project root only
//! over the baseline they were seeded from, all or none, once.
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::write_coordinator::patch_manifest::PATCH_MANIFEST_SCHEMA;
use crate::write_coordinator::project_inputs::{InputChange, write_json, write_test_policy};
use crate::write_coordinator::{ItemId, ManifestStatus};

const REGISTRY: &str = ".archon/lab/data/registry.json";
const INDEX: &str = ".archon/lab/data/index.json";

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    run_root: PathBuf,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let run_root = root.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    write_test_policy(&run_root, &root, &[".archon/lab"]);
    std::fs::create_dir_all(root.join(".archon/lab/data")).unwrap();
    std::fs::write(root.join(REGISTRY), "v1").unwrap();
    Project {
        _dir: dir,
        root,
        run_root,
    }
}

fn manifest(item: &str) -> PatchManifest {
    PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.into(),
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from(item),
        baseline_commit: "base".into(),
        patch_path: PathBuf::from("unused.patch"),
        declared_target_files: vec![],
        changed_files: vec![],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::IdempotentNoop,
        skipped_ignored: BTreeMap::new(),
        materialized: BTreeMap::new(),
        destination_baselines: BTreeMap::new(),
        needs_attention: None,
        materializable: Default::default(),
    }
}

/// The capture a branch seeded from `baseline` left: `changes` of
/// (path, new bytes or `None` for a deletion).
fn capture(p: &Project, item: &str, changes: &[(&str, &str, Option<&str>)]) {
    let mut record = CaptureRecord {
        task_ids: vec![format!("TASK-{item}")],
        changes: BTreeMap::new(),
    };
    for (rel, baseline, post) in changes {
        let post = match post {
            Some(bytes) => {
                let kept = captured_bytes_dir(&p.run_root, "impl", item).join(rel);
                std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
                std::fs::write(kept, bytes).unwrap();
                hash(bytes.as_bytes())
            }
            None => "deleted".to_string(),
        };
        let baseline = if *baseline == "absent" {
            "absent".into()
        } else {
            hash(baseline.as_bytes())
        };
        record
            .changes
            .insert(rel.to_string(), InputChange { baseline, post });
    }
    write_json(&capture_path(&p.run_root, "impl", item), &record).unwrap();
}

fn read(p: &Project, rel: &str) -> String {
    std::fs::read_to_string(p.root.join(rel)).unwrap_or_else(|_| "<absent>".into())
}

#[test]
fn two_branches_of_one_wave_change_a_shared_input_and_the_second_is_refused_stale() {
    let p = project();
    capture(&p, "a", &[(REGISTRY, "v1", Some("v2-from-a"))]);
    capture(
        &p,
        "b",
        &[
            (REGISTRY, "v1", Some("v2-from-b")),
            (INDEX, "absent", Some("i")),
        ],
    );
    assert_eq!(apply(&p.run_root, &manifest("a")), None);
    assert_eq!(read(&p, REGISTRY), "v2-from-a");
    // What it replaced is kept.
    let kept = p
        .run_root
        .join("write-coordination/project-inputs-replaced/impl/a")
        .join(REGISTRY);
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "v1");
    let refused = apply(&p.run_root, &manifest("b")).expect("refused");
    assert!(
        refused.contains("stale baseline at .archon/lab/data/registry.json"),
        "{refused}"
    );
    // All or none: b's other change was not left behind, and a's stands.
    assert_eq!(read(&p, REGISTRY), "v2-from-a");
    assert_eq!(read(&p, INDEX), "<absent>");
    let log = run_project_input_landings(&p.run_root).unwrap();
    let outcomes: Vec<(&str, &str, &str)> = log
        .iter()
        .map(|l| (l.item_id.as_str(), l.path.as_str(), l.outcome.as_str()))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("a", REGISTRY, "applied"),
            ("b", INDEX, "refused"),
            ("b", REGISTRY, "refused"),
        ]
    );
    assert_eq!(log[2].task_ids, ["TASK-b"]);
}

#[test]
fn a_failure_part_way_puts_every_copy_back() {
    let p = project();
    // The second change's kept bytes do not match its capture.
    capture(
        &p,
        "a",
        &[(INDEX, "absent", Some("i")), (REGISTRY, "v1", Some("v2"))],
    );
    std::fs::write(
        captured_bytes_dir(&p.run_root, "impl", "a").join(REGISTRY),
        "tampered",
    )
    .unwrap();
    let refused = apply(&p.run_root, &manifest("a")).expect("refused");
    assert!(refused.contains("do not match"), "{refused}");
    assert_eq!(read(&p, INDEX), "<absent>");
    assert_eq!(read(&p, REGISTRY), "v1");
}

#[test]
fn an_applied_capture_moves_nothing_again_and_a_deletion_lands() {
    let p = project();
    capture(&p, "a", &[(REGISTRY, "v1", None)]);
    assert_eq!(apply(&p.run_root, &manifest("a")), None);
    assert_eq!(read(&p, REGISTRY), "<absent>");
    // A later landing wrote the file again; a resume re-applying `a`'s
    // capture neither deletes it nor reports it stale.
    std::fs::write(p.root.join(REGISTRY), "v3").unwrap();
    assert_eq!(apply(&p.run_root, &manifest("a")), None);
    assert_eq!(read(&p, REGISTRY), "v3");
    assert_eq!(run_project_input_landings(&p.run_root).unwrap().len(), 1);
}

#[test]
fn an_engine_namespace_is_never_written_and_the_refusal_is_recorded() {
    let p = project();
    write_test_policy(&p.run_root, &p.root, &[".archon"]);
    capture(&p, "a", &[(".archon/agents/coder.md", "absent", Some("x"))]);
    let mut rec = super::super::ApplyRecord {
        wave_id: 0,
        started_at: std::time::SystemTime::now(),
        completed_at: std::time::SystemTime::now(),
        items_applied: vec![],
        items_failed: vec![],
        verify_result: None,
        project_input_refusals: vec![],
    };
    land(&p.run_root, &p.root, &manifest("a"), false, &mut rec);
    assert_eq!(rec.project_input_refusals.len(), 1);
    assert!(
        rec.project_input_refusals[0]
            .1
            .contains("namespace the engine loads from")
    );
    assert!(!p.root.join(".archon/agents/coder.md").exists());
}

fn git(root: &std::path::Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn a_landed_tracked_input_is_brought_into_the_project_root_and_a_divergent_copy_kept() {
    let p = project();
    let repo = p._dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".archon/lab")).unwrap();
    git(&repo, &["init", "-q"]);
    for (rel, body) in [
        (".archon/lab/spec.json", "s1"),
        (".archon/lab/other.json", "o1"),
    ] {
        std::fs::write(repo.join(rel), body).unwrap();
        std::fs::write(p.root.join(rel), body).unwrap();
    }
    git(&repo, &["add", "-A"]);
    git(
        &repo,
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
    let base = git(&repo, &["rev-parse", "HEAD"]);
    // The landing changed both; the project's copy of `other` had moved on.
    std::fs::write(repo.join(".archon/lab/spec.json"), "s2").unwrap();
    std::fs::write(repo.join(".archon/lab/other.json"), "o2").unwrap();
    std::fs::write(p.root.join(".archon/lab/other.json"), "o-edited").unwrap();
    let mut m = manifest("a");
    m.baseline_commit = base;
    m.changed_files = vec![
        ".archon/lab/spec.json".into(),
        ".archon/lab/other.json".into(),
    ];
    assert_eq!(sync_tracked(&p.run_root, &repo, &m), None);
    assert_eq!(read(&p, ".archon/lab/spec.json"), "s2");
    assert_eq!(read(&p, ".archon/lab/other.json"), "o2");
    let kept = p
        .run_root
        .join("write-coordination/project-inputs-replaced/impl/a/.archon/lab/other.json");
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "o-edited");
    let log = run_project_input_landings(&p.run_root).unwrap();
    assert!(log.iter().all(|line| line.outcome == "synced"), "{log:?}");
    assert!(log[0].reason.contains("kept at"), "{log:?}");
}
