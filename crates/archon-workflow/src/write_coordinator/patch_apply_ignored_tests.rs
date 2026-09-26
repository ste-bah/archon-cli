use super::*;
#[test]
fn ignored_deliverable_stays_out_of_shared_tree_and_is_reported() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join(".gitignore"), "docs/\n.archon/\n").unwrap();
    git(&["add", ".gitignore"], root);
    git(&["commit", "-qm", "ignore generated files"], root);
    let (mut manifest, pre) = prepare(
        root,
        "ignored",
        &["docs/report.md"],
        &[("docs/report.md", "report bytes")],
    );
    manifest.status = ManifestStatus::IdempotentNoop;
    let run_root = root.join(".archon/workflows/run1");
    let record = with_repo_lock(root, || {
        apply_wave(
            root,
            &[manifest.clone()],
            &BTreeMap::from([("ignored".into(), pre)]),
            1,
            &run_root,
            "run1",
            "impl",
        )
    })
    .unwrap();
    assert!(record.items_failed.is_empty(), "{:?}", record.items_failed);
    assert!(record.items_applied.is_empty());
    assert!(
        !root.join("docs/report.md").exists(),
        "ignored output contaminated canonical"
    );
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(run_root.join("write-coordination/stages/impl/manifests/ignored.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["status"]["status"], "skipped_ignored");
    assert_eq!(
        std::fs::read(run_root.join("artifacts/ignored-deliverables/impl/ignored/docs/report.md"))
            .unwrap(),
        b"report bytes"
    );
}

/// Issue-113, with the project root the repository itself: an ignored
/// PROJECT artifact the branch regenerated lands at the path verifiers read,
/// beside a tracked change that lands as a commit.
#[test]
fn ignored_project_artifact_lands_where_it_is_verified() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join(".gitignore"), ".archon/*\ndocs/\n").unwrap();
    git(&["add", ".gitignore"], root);
    git(&["commit", "-qm", "ignore generated files"], root);
    let artifact = ".archon/lab/out.json";
    let (ignored_only, ignored_pre) = prepare(
        root,
        "ignored",
        &[artifact, "docs/report.md"],
        &[(artifact, "{\"v\":2}"), ("docs/report.md", "report")],
    );
    let (mixed, mixed_pre) = prepare(
        root,
        "mixed",
        &["src/lib.rs", ".archon/lab/mixed.json"],
        &[("src/lib.rs", "// new\n"), (".archon/lab/mixed.json", "m")],
    );
    assert_eq!(ignored_only.status, ManifestStatus::SkippedIgnored);
    let deliverables = std::collections::BTreeSet::from([
        artifact.to_string(),
        ".archon/lab/mixed.json".to_string(),
        "docs/report.md".to_string(),
    ]);
    let (mut ignored_only, mut mixed) = (ignored_only, mixed);
    ignored_only.materializable = deliverables.clone();
    mixed.materializable = deliverables;
    let run_root = root.join(".archon/workflows/run1");
    let record = with_repo_lock(root, || {
        apply_wave(
            root,
            &[ignored_only.clone(), mixed.clone()],
            &BTreeMap::from([("ignored".into(), ignored_pre), ("mixed".into(), mixed_pre)]),
            1,
            &run_root,
            "run1",
            "impl",
        )
    })
    .unwrap();
    assert!(record.items_failed.is_empty(), "{:?}", record.items_failed);
    assert_eq!(std::fs::read(root.join(artifact)).unwrap(), b"{\"v\":2}");
    assert_eq!(
        std::fs::read(root.join(".archon/lab/mixed.json")).unwrap(),
        b"m"
    );
    assert!(
        !root.join("docs/report.md").exists(),
        "a repository-rooted ignored output stays a run artifact"
    );
    let saved = |item: &str| -> PatchManifest {
        serde_json::from_slice(
            &std::fs::read(run_root.join(format!(
                "write-coordination/stages/impl/manifests/{item}.json"
            )))
            .unwrap(),
        )
        .unwrap()
    };
    let ignored = saved("ignored");
    assert_eq!(ignored.status, ManifestStatus::SkippedIgnored);
    let receipt = &ignored.materialized[artifact];
    assert_eq!(
        receipt.destination,
        root.join(artifact).display().to_string()
    );
    assert_eq!(receipt.pre_hash, "absent");
    assert_eq!(receipt.post_hash, blake3_of(&root.join(artifact)));
    assert!(!ignored.materialized.contains_key("docs/report.md"));
    let mixed = saved("mixed");
    assert_eq!(mixed.status, ManifestStatus::Applied);
    assert_eq!(
        mixed.post_hashes[".archon/lab/mixed.json"],
        mixed.materialized[".archon/lab/mixed.json"].post_hash,
        "the landing's post-state includes the copy"
    );
    let receipts: Vec<u64> = [&ignored, &mixed]
        .iter()
        .flat_map(|m| m.materialized.values().map(|r| r.sequence))
        .collect();
    assert_eq!(receipts.len(), 2);
    assert_ne!(receipts[0], receipts[1], "one run-wide order");
    let committed = std::process::Command::new("git")
        .current_dir(root)
        .args(["show", "--name-only", "--format=", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&committed.stdout).trim(),
        "src/lib.rs"
    );
}

/// A patch that does not apply puts back every copy its landing made, and
/// the failed manifest records none.
#[test]
fn a_patch_that_fails_to_apply_undoes_its_materialization() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join(".gitignore"), ".archon/*\n").unwrap();
    git(&["add", ".gitignore"], root);
    git(&["commit", "-qm", "ignore generated files"], root);
    let artifact = ".archon/lab/mixed.json";
    let (mut mixed, pre) = prepare(
        root,
        "mixed",
        &["src/lib.rs", artifact],
        &[("src/lib.rs", "// new\n"), (artifact, "m")],
    );
    mixed.materializable = std::collections::BTreeSet::from([artifact.to_string()]);
    std::fs::write(&mixed.patch_path, "not a patch\n").unwrap();
    let run_root = root.join(".archon/workflows/run1");
    let record = with_repo_lock(root, || {
        apply_wave(
            root,
            &[mixed.clone()],
            &BTreeMap::from([("mixed".into(), pre)]),
            1,
            &run_root,
            "run1",
            "impl",
        )
    });
    let failed = match record {
        Ok(record) => !record.items_failed.is_empty(),
        Err(_) => true,
    };
    assert!(failed, "the patch was refused");
    assert!(!root.join(artifact).exists(), "the copy was undone");
    let saved: PatchManifest = serde_json::from_slice(
        &std::fs::read(run_root.join("write-coordination/stages/impl/manifests/mixed.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(saved.materialized.is_empty(), "{saved:#?}");
}
