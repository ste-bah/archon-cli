use super::*;
#[test]
fn classification_lock_bytes_remain_identical() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/classification");
    let mut count = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "lock") {
            let bytes = std::fs::read(&path).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let serialized = if path.file_stem().unwrap() == "acceptance" {
                serde_json::to_vec_pretty(
                    &serde_json::from_value::<crate::task_set_contract::AcceptanceLock>(value)
                        .unwrap(),
                )
                .unwrap()
            } else {
                serde_json::to_vec_pretty(
                    &serde_json::from_value::<crate::task_skeleton::TaskSkeletonLock>(value)
                        .unwrap(),
                )
                .unwrap()
            };
            assert_eq!(serialized, bytes);
            let temp = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp.path(), &serialized).unwrap();
            assert_eq!(std::fs::read(temp.path()).unwrap(), bytes);
            count += 1;
        }
    }
    assert!(count > 0);
}
#[test]
fn strict_records_and_stability() {
    assert!(
        serde_json::from_str::<MutantSet>(r#"{"schema":1,"base_commit":"a","extra":0}"#).is_err()
    );
    let mut c = ControlRecord {
        check_id: "a".into(),
        check_digest: "b".into(),
        pre: RunOutcome::Passed { elapsed_ms: 1 },
        post: None,
        restored_runs: 0,
    };
    assert!(!c.stable());
    c.post = Some(RunOutcome::Passed { elapsed_ms: 1 });
    assert!(c.stable());
    c.pre = RunOutcome::NoExitStatus;
    assert!(!c.stable());
}
#[test]
fn validation_quotas_duplicates_and_unknown_pairs() {
    use crate::v2::requirement_classes::RequirementClassification;
    let lock: crate::task_set_contract::AcceptanceLock = serde_json::from_slice(include_bytes!(
        "../tests/fixtures/classification/acceptance.lock"
    ))
    .unwrap();
    let classes = RequirementClassification { schema_version:2, baseline_commit:lock.baseline_commit.clone(),acceptance_digest:lock.digest.clone(),baseline_results_digest:String::new(),error:None,base_snapshot:None,entries:vec![serde_json::from_value(serde_json::json!({"task_id":"task","id":"item","class":"candidate_done","reason":null,"fallback":null,"checks":{},"deliverables":[],"text_item":false,"passing_check_without_deliverable":false})).unwrap()] };
    let entry = classes.entries[0].clone();
    let m = SuppliedMutant {
        task_id: entry.task_id,
        requirement_id: entry.id,
        kind: MutantKind::Behaviour,
        patch: "first".into(),
    };
    let mut set = MutantSet {
        schema: BREAKAGE_SCHEMA,
        base_commit: lock.baseline_commit.clone().unwrap(),
        mutants: vec![m.clone(), m.clone()],
    };
    let (kept, refused) = selection::validate(&set, &lock, &classes).unwrap();
    assert_eq!(kept.len(), 1);
    assert!(refused[0].ends_with(": patch_duplicate"));
    set.mutants = (0..6)
        .map(|i| {
            let mut m = m.clone();
            m.patch = i.to_string();
            m
        })
        .collect();
    let (kept, refused) = selection::validate(&set, &lock, &classes).unwrap();
    assert_eq!(kept.len(), 5);
    assert!(refused[0].ends_with(": mutant_quota_exceeded"));
    for m in &mut set.mutants {
        m.kind = MutantKind::BodyGut;
    }
    let (kept, refused) = selection::validate(&set, &lock, &classes).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(refused.len(), 5);
    for (patch, code) in [
        (String::new(), "patch_empty"),
        ("x".repeat(256 * 1024 + 1), "patch_too_large"),
    ] {
        let mut m = m.clone();
        m.patch = patch;
        set.mutants = vec![m];
        assert!(selection::validate(&set, &lock, &classes).unwrap().1[0].ends_with(code));
    }
    set.mutants[0].task_id = "unknown".into();
    assert!(
        selection::validate(&set, &lock, &classes).unwrap().1[0].ends_with("mutant_unknown_pair")
    );
    set.schema = 2;
    assert_eq!(
        selection::validate(&set, &lock, &classes).unwrap_err(),
        "mutant_set_schema"
    );
    set.schema = 1;
    set.base_commit = "different".into();
    assert_eq!(
        selection::validate(&set, &lock, &classes).unwrap_err(),
        "mutant_commit_mismatch"
    );
}
#[test]
fn attribution_is_strict_for_files_trees_and_shared_paths() {
    use crate::base_snapshot::SnapshotKind::{Blob, Tree};
    let countable = vec![("one".into(), Blob), ("dir".into(), Tree)];
    let shared = vec![("shared".into(), Tree)];
    for touched in [
        vec!["one".into()],
        vec!["dir/child".into()],
        vec!["one".into(), "dir/child".into()],
    ] {
        assert_eq!(selection::attribution(&touched, &countable, &shared), None);
    }
    for touched in [
        vec!["outside".into()],
        vec!["one/child".into()],
        vec!["one".into(), "outside".into()],
    ] {
        assert_eq!(
            selection::attribution(&touched, &countable, &shared),
            Some(NotCounted::TouchesOutsideDeliverables)
        );
    }
    assert_eq!(
        selection::attribution(&["one".into(), "shared/child".into()], &countable, &shared),
        Some(NotCounted::TouchesShared)
    );
}

// Keep the existing selection assertions while adapting the engine input shape.
mod selection {
    pub(super) use super::super::selection::attribution;

    pub(super) fn validate(
        set: &super::MutantSet,
        lock: &crate::task_set_contract::AcceptanceLock,
        classes: &crate::v2::requirement_classes::RequirementClassification,
    ) -> Result<(Vec<super::SuppliedMutant>, Vec<String>), &'static str> {
        let pairs = classes
            .entries
            .iter()
            .map(|e| (e.task_id.clone(), e.id.clone()))
            .collect::<Vec<_>>();
        super::super::selection::validate(set, lock.baseline_commit.as_deref().unwrap(), &pairs)
    }
}
