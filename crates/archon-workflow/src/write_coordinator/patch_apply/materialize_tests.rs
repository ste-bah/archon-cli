//! Issue-113: which declared ignored targets are copied, where, and what a
//! refusal leaves behind. A project directory separate from any repository,
//! as on the live run: `<project>/.archon/workflows/<run>` is the run root.
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::write_coordinator::ItemId;
use crate::write_coordinator::patch_manifest::PATCH_MANIFEST_SCHEMA;

const PINE: &str = ".archon/lab/strategy/out.pine";

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
    Project {
        _dir: dir,
        root,
        run_root,
    }
}

/// A manifest whose capture carried `captured` as ignored bytes for each
/// path, with the given pre-hash (the branch's own baseline).
fn manifest(p: &Project, item: &str, captured: &[(&str, &[u8], &str)]) -> PatchManifest {
    let patch_path = p.run_root.join(format!(
        "write-coordination/stages/impl/patches/{item}.patch"
    ));
    let sidecar = patch_path.with_extension("ignored");
    let (mut pre, mut post) = (BTreeMap::new(), BTreeMap::new());
    for (rel, bytes, pre_hash) in captured {
        let path = sidecar.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        pre.insert(rel.to_string(), pre_hash.to_string());
        post.insert(rel.to_string(), hash(bytes));
    }
    let materializable = post.keys().cloned().collect();
    PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.into(),
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from(item),
        baseline_commit: "base".into(),
        patch_path,
        declared_target_files: post.keys().cloned().collect(),
        changed_files: vec![],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: pre,
        post_hashes: post,
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::SkippedIgnored,
        skipped_ignored: BTreeMap::new(),
        materialized: BTreeMap::new(),
        materializable,
    }
}

fn persist(p: &Project, m: &PatchManifest) {
    let path = p
        .run_root
        .join(format!(
            "write-coordination/stages/{}/manifests",
            m.stage_id
        ))
        .join(format!("{}.json", m.item_id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(m).unwrap()).unwrap();
}

#[test]
fn a_changed_project_artifact_is_copied_to_the_path_verifiers_read() {
    let p = project();
    let legacy = p.root.join(PINE);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "legacy render").unwrap();
    let mut m = manifest(&p, "fix-0", &[(PINE, b"regenerated", "absent")]);

    materialize(&p.run_root, &mut m).expect("materialized");

    assert_eq!(std::fs::read(&legacy).unwrap(), b"regenerated");
    let receipt = &m.materialized[PINE];
    // Exactly the path the verification prompts are stamped with.
    let stamped = crate::v2::project_artifact_stamping::project_artifact_destination(
        &p.root.display().to_string(),
        PINE,
    );
    assert_eq!(Some(receipt.destination.clone()), stamped);
    assert_eq!(receipt.pre_hash, hash(b"legacy render"));
    assert_eq!(receipt.post_hash, hash(b"regenerated"));
    assert_eq!(receipt.sequence, 1);
}

#[test]
fn only_changed_declared_deliverables_in_a_namespace_move() {
    let p = project();
    let same = hash(b"unchanged");
    let mut m = manifest(
        &p,
        "fix-0",
        &[
            (".archon/lab/unchanged.json", b"unchanged", same.as_str()),
            ("docs/report.md", b"repository-rooted", "absent"),
            (".archon/workflows/run1/state.json", b"{}", "absent"),
            (".archon/Workflows/run1/other.json", b"{}", "absent"),
            (".archon/hooks.toml", b"[hooks]", "absent"),
            (".archon/lab/undeclared.json", b"{}", "absent"),
        ],
    );
    m.materializable.remove(".archon/lab/undeclared.json");
    // A sidecar file the manifest does not declare is never read.
    let stray = m
        .patch_path
        .with_extension("ignored")
        .join(".archon/lab/stray");
    std::fs::write(&stray, "stray").unwrap();

    materialize(&p.run_root, &mut m).expect("nothing to refuse");

    assert!(m.materialized.is_empty(), "{:?}", m.materialized);
    assert!(!p.root.join(".archon/lab/unchanged.json").exists());
    assert!(
        !p.root.join("docs/report.md").exists(),
        "repository paths stay run artifacts"
    );
    assert!(!p.root.join(".archon/lab/stray").exists());
    assert!(
        !p.run_root.join("state.json").exists(),
        "never into the run store"
    );
    assert!(
        !p.run_root.join("other.json").exists(),
        "compared case-blind"
    );
    assert!(
        !p.root.join(".archon/hooks.toml").exists(),
        "never engine config"
    );
    assert!(
        !p.root.join(".archon/lab/undeclared.json").exists(),
        "only deliverables"
    );
}

#[test]
fn a_leftover_sidecar_the_capture_recorded_as_deleted_is_not_copied() {
    let p = project();
    let mut m = manifest(&p, "fix-0", &[(PINE, b"from an earlier capture", "absent")]);
    m.post_hashes.insert(PINE.into(), "deleted".into());

    materialize(&p.run_root, &mut m).expect("skipped");

    assert!(m.materialized.is_empty());
    assert!(!p.root.join(PINE).exists());
}

#[test]
fn bytes_the_capture_did_not_vouch_for_refuse_and_undo_every_copy() {
    let p = project();
    let first = ".archon/lab/a.json";
    let mut m = manifest(
        &p,
        "fix-0",
        &[(first, b"a", "absent"), (PINE, b"b", "absent")],
    );
    let tampered = m.patch_path.with_extension("ignored").join(PINE);
    std::fs::write(&tampered, "swapped after capture").unwrap();

    let error = materialize(&p.run_root, &mut m).expect_err("refused");

    assert!(error.contains("post-hash"), "{error}");
    assert!(!p.root.join(first).exists(), "the earlier copy was undone");
    assert!(!p.root.join(PINE).exists());
    assert!(m.materialized.is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlink_on_the_way_to_the_destination_is_refused_not_followed() {
    let p = project();
    let elsewhere = p.root.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(p.root.join(".archon")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, p.root.join(".archon/lab")).unwrap();
    let mut m = manifest(&p, "fix-0", &[(PINE, b"regenerated", "absent")]);

    let error = materialize(&p.run_root, &mut m).expect_err("refused");

    assert!(error.contains("symlink"), "{error}");
    assert!(!elsewhere.join("strategy/out.pine").exists());
}

#[test]
fn the_sequence_continues_from_the_runs_landed_receipts() {
    let p = project();
    let mut earlier = manifest(&p, "fix-0", &[(PINE, b"one", "absent")]);
    materialize(&p.run_root, &mut earlier).unwrap();
    persist(&p, &earlier);
    // A failed landing's receipts never count.
    let mut failed = manifest(&p, "fix-9", &[(".archon/lab/x", b"x", "absent")]);
    failed.materialized.insert(
        ".archon/lab/x".into(),
        MaterializedDeliverable {
            destination: "/nowhere".into(),
            pre_hash: "absent".into(),
            post_hash: hash(b"x"),
            sequence: 99,
        },
    );
    failed.status = ManifestStatus::Failed { reason: "x".into() };
    persist(&p, &failed);

    let mut later = manifest(&p, "fix-1", &[(PINE, b"two", "absent")]);
    materialize(&p.run_root, &mut later).unwrap();

    assert_eq!(earlier.materialized[PINE].sequence, 1);
    assert_eq!(later.materialized[PINE].sequence, 2);
    assert_eq!(later.materialized[PINE].pre_hash, hash(b"one"));
}

#[test]
fn a_planted_temporary_file_is_refused_not_written_through() {
    let p = project();
    let destination = p.root.join(PINE);
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let planted = destination.parent().unwrap().join(format!(
        ".out.pine.{}.archon-materialize.tmp",
        std::process::id()
    ));
    std::fs::write(&planted, "planted").unwrap();
    let mut m = manifest(&p, "fix-0", &[(PINE, b"regenerated", "absent")]);

    assert!(materialize(&p.run_root, &mut m).is_err());
    assert_eq!(std::fs::read(&planted).unwrap(), b"planted");
    assert!(!destination.exists());
}

#[test]
fn universe_deliverables_are_the_concrete_contract_paths() {
    use crate::task_universe::{
        WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
    };
    let contract = |path: &str| WorkflowV2DeliverableContract {
        artifact_path: path.into(),
        ..Default::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "t".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            deliverable_contracts: vec![
                contract("./.archon/lab/a.json"),
                contract(".archon/lab/<id>/b.json"),
                contract("/abs/c.json"),
                contract(" src/lib.rs "),
            ],
            ..Default::default()
        }],
    };
    let set = universe_deliverables(&universe);
    assert_eq!(
        set.into_iter().collect::<Vec<_>>(),
        vec![".archon/lab/a.json".to_string(), "src/lib.rs".to_string()]
    );
}
