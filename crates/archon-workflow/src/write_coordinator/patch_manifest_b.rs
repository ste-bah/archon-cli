fn path_overlaps(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Write `<stage>/manifests/<item>.json` + `<stage>/patches/<item>.patch` atomically.
pub fn persist_manifest(
    run_root: &Path,
    run_id: &str,
    stage_id: &str,
    item_id: &ItemId,
    captured: &CapturedPatch,
    status: ManifestStatus,
) -> Result<PathBuf, PatchError> {
    let (manifest_path, patch_path) = manifest_paths(run_root, stage_id, item_id);
    create_parents(&manifest_path)?;
    create_parents(&patch_path)?;
    write_atomic(&patch_path, &captured.patch_bytes)?;

    let declared: Vec<String> = captured.post_hashes.keys().cloned().collect();
    let manifest = PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.to_string(),
        run_id: run_id.to_string(),
        stage_id: stage_id.to_string(),
        item_id: item_id.clone(),
        baseline_commit: captured.baseline_commit.clone(),
        patch_path: patch_path.clone(),
        declared_target_files: declared,
        changed_files: captured.changed_files.clone(),
        created_files: captured.created_files.clone(),
        deleted_files: captured.deleted_files.clone(),
        pre_hashes: captured.pre_hashes.clone(),
        post_hashes: captured.post_hashes.clone(),
        verify_command: None,
        agent_artifact_path: None,
        status,
    };
    write_manifest_json(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

/// Rewrite ONLY the manifest JSON, leaving the patch file untouched.
pub fn persist_manifest_status_update(
    run_root: &Path,
    _run_id: &str,
    stage_id: &str,
    item_id: &ItemId,
    manifest: &PatchManifest,
) -> Result<(), PatchError> {
    let (manifest_path, _patch_path) = manifest_paths(run_root, stage_id, item_id);
    create_parents(&manifest_path)?;
    write_manifest_json(&manifest_path, manifest)
}

fn manifest_paths(run_root: &Path, stage_id: &str, item_id: &ItemId) -> (PathBuf, PathBuf) {
    let stage_root = run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id);
    let manifest = stage_root.join("manifests").join(format!("{item_id}.json"));
    let patch = stage_root.join("patches").join(format!("{item_id}.patch"));
    (manifest, patch)
}

fn create_parents(path: &Path) -> Result<(), PatchError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PatchError::PersistFailed { source })?;
    }
    Ok(())
}

fn write_manifest_json(path: &Path, manifest: &PatchManifest) -> Result<(), PatchError> {
    let json = serde_json::to_vec_pretty(manifest)
        .map_err(|e| PatchError::PersistFailed { source: e.into() })?;
    write_atomic(path, &json)
}

/// Write to `<path>.tmp` then rename — atomic on POSIX.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PatchError> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, bytes).map_err(|source| PatchError::PersistFailed { source })?;
    std::fs::rename(&tmp, path).map_err(|source| PatchError::PersistFailed { source })?;
    Ok(())
}

#[cfg(test)]
#[path = "patch_manifest_d15_tests.rs"]
mod d15_tests;
#[cfg(test)]
#[path = "patch_manifest_line_count_tests.rs"]
mod line_count_tests;
#[cfg(test)]
#[path = "patch_manifest_tests.rs"]
mod tests;
