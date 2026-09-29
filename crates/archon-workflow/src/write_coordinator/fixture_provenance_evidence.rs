//! Where a refused landing is kept (split from `fixture_provenance` for the
//! 500-line ceiling).

use std::path::{Path, PathBuf};

use super::FixtureHit;

/// Where a refused landing's bytes and findings are kept as evidence.
pub fn evidence_dir(run_root: &Path, stage_id: &str, item_id: &str) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("project-inputs-refused")
        .join(stage_id)
        .join(item_id)
}

/// Keep `files` (relative path, bytes) and the findings of a refused
/// landing. Best effort: the refusal stands whether or not this succeeds.
pub fn keep_evidence(
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    hits: &[FixtureHit],
    files: &[(String, Vec<u8>)],
) {
    let dir = evidence_dir(run_root, stage_id, item_id);
    let clean = |rel: &str| {
        Path::new(rel)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    };
    for (rel, bytes) in files.iter().filter(|(rel, _)| clean(rel)) {
        let path = dir.join(rel);
        let kept = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, bytes));
        if let Err(error) = kept {
            eprintln!("fixture provenance: {} not kept: {error}", path.display());
        }
    }
    let findings: Vec<String> = hits.iter().map(FixtureHit::finding).collect();
    let record = serde_json::json!({ "findings": findings });
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::write(
            dir.join("fixture-findings.json"),
            serde_json::to_vec_pretty(&record).unwrap_or_default(),
        )
    });
    if let Err(error) = written {
        eprintln!("fixture provenance: findings for {stage_id}/{item_id} not kept: {error}");
    }
}
