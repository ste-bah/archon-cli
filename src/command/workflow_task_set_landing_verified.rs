use archon_workflow::base_snapshot::{
    BaseSnapshot, SnapshotEntry, UnverifiedReason, snapshot_path_key,
};
use archon_workflow::write_coordinator::patch_manifest::{ManifestStatus, PatchManifest};
use std::path::Path;
pub(super) fn mark(project: &Path, tasks: &Path, repository: &Path, snapshot: &mut BaseSnapshot) {
    let store = archon_workflow::WorkflowStore::project(project);
    let mut digests = std::collections::BTreeSet::new();
    for run in store.list_runs().unwrap_or_default() {
        let Ok(launch) = crate::command::acceptance_chain::launch_snapshot(&store, &run.id) else {
            continue;
        };
        if Path::new(&launch.canonical_task_root_identity)
            .canonicalize()
            .ok()
            == tasks.canonicalize().ok()
        {
            for manifest in archon_workflow::write_coordinator::patch_manifest::applied_manifests(
                &store.run_dir(&run.id),
            ) {
                digests.extend(manifest_digests(repository, manifest));
            }
        }
    }
    confirm(snapshot, &digests);
}
fn manifest_digests(repository: &Path, manifest: PatchManifest) -> Vec<(String, String)> {
    if manifest.schema != archon_workflow::write_coordinator::patch_manifest::PATCH_MANIFEST_SCHEMA
        || manifest.status != ManifestStatus::Applied
    {
        return Vec::new();
    }
    manifest
        .post_hashes
        .into_iter()
        .chain(
            manifest
                .materialized
                .into_iter()
                .map(|(path, value)| (path, value.post_hash)),
        )
        .map(|(path, digest)| (snapshot_path_key(repository, &path), digest))
        .collect()
}
fn confirm(snapshot: &mut BaseSnapshot, digests: &std::collections::BTreeSet<(String, String)>) {
    for (key, entry) in &snapshot.entries {
        if let SnapshotEntry::Unverified {
            reason: UnverifiedReason::Ignored | UnverifiedReason::Untracked,
            live_digest: Some(digest),
            ..
        } = entry
            && digests.contains(&(key.clone(), digest.clone()))
        {
            snapshot
                .landing_verified
                .insert(key.clone(), digest.clone());
        }
    }
    snapshot.digest = snapshot.recomputed_digest();
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn manifest(status: serde_json::Value) -> PatchManifest {
        serde_json::from_value(serde_json::json!({"schema":archon_workflow::write_coordinator::patch_manifest::PATCH_MANIFEST_SCHEMA,"run_id":"run","stage_id":"stage","item_id":"item","baseline_commit":"base","patch_path":"patch","declared_target_files":["artifact"],"changed_files":["artifact"],"created_files":[],"deleted_files":[],"pre_hashes":{},"post_hashes":{"artifact":"expected"},"verify_command":null,"agent_artifact_path":null,"status":status})).unwrap()
    }
    fn snapshot() -> BaseSnapshot {
        let mut s = BaseSnapshot {
            landing_verified: Default::default(),
            commit: "base".into(),
            entries: [(
                "artifact".into(),
                SnapshotEntry::Unverified {
                    reason: UnverifiedReason::Ignored,
                    live_digest: Some("expected".into()),
                    live_note: None,
                },
            )]
            .into(),
            digest: String::new(),
        };
        s.digest = s.recomputed_digest();
        s
    }
    #[test]
    fn landing_verified_requires_applied_and_matching_digest() {
        for (status, digest, expected) in [
            (serde_json::json!({"status":"applied"}), "expected", true),
            (
                serde_json::json!({"status":"failed","reason":"fault"}),
                "expected",
                false,
            ),
            (serde_json::json!({"status":"applied"}), "changed", false),
        ] {
            let mut m = manifest(status);
            m.post_hashes.insert("artifact".into(), digest.into());
            let digests = manifest_digests(Path::new(""), m).into_iter().collect();
            let mut s = snapshot();
            confirm(&mut s, &digests);
            assert_eq!(s.landing_verified.contains_key("artifact"), expected);
            assert!(s.valid_digest());
        }
    }
    #[test]
    fn landing_verified_uses_materialized_declared_keys_and_keeps_all_matches() {
        let mut m = manifest(serde_json::json!({"status":"applied"}));
        m.post_hashes.clear();
        m.materialized.insert(
            "artifact".into(),
            archon_workflow::write_coordinator::patch_manifest::MaterializedDeliverable {
                destination: "destination".into(),
                pre_hash: "absent".into(),
                post_hash: "expected".into(),
                sequence: 1,
            },
        );
        let mut digests: std::collections::BTreeSet<_> =
            manifest_digests(Path::new(""), m).into_iter().collect();
        digests.insert(("artifact".into(), "older".into()));
        let mut s = snapshot();
        confirm(&mut s, &digests);
        assert!(s.landing_verified.contains_key("artifact"));
    }
}
#[cfg(test)]
mod record_path_tests {
    use super::tests::manifest;
    #[test]
    fn landing_verified_reads_only_applied_coordinator_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let path = root.join("write-coordination/stages/stage/manifests");
        std::fs::create_dir_all(&path).unwrap();
        let applied = manifest(serde_json::json!({"status":"applied"}));
        let failed = manifest(serde_json::json!({"status":"failed","reason":"fault"}));
        std::fs::write(
            path.join("applied.json"),
            serde_json::to_vec(&applied).unwrap(),
        )
        .unwrap();
        std::fs::write(
            path.join("failed.json"),
            serde_json::to_vec(&failed).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("authored.json"),
            serde_json::to_vec(&applied).unwrap(),
        )
        .unwrap();
        std::fs::write(path.join("broken.json"), b"broken").unwrap();
        let records = archon_workflow::write_coordinator::patch_manifest::applied_manifests(root);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], applied);
    }
}
