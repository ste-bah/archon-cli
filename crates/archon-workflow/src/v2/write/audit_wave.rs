//! Apply credit and refresh classification use host receipts, never agent claims.
use super::*;
use crate::repository_audit::runtime::Snapshot;

pub(super) async fn after_apply(ctx:&WorktreePlanRunContext<'_>,artifacts:&WorktreeWaveArtifacts)->WorkflowResult<()> {
    let Some(audit)=ctx.dispatch.repository_audit() else{return Ok(());};
    let Some((receipt, commit)) = &artifacts.applied_receipt else { return Ok(()); };
    let mut applied=Vec::new();
    for manifest in &artifacts.manifests {
        if !receipt.items_applied.contains(&manifest.item_id) { continue; }
        let path=PathBuf::from(manifest_path_for(&ctx.setup.run_root,&ctx.execution.call.id,&manifest.item_id));
        let persisted:PatchManifest=serde_json::from_slice(&std::fs::read(&path).map_err(|e|WorkflowError::io(&path,e))?)?;
        if persisted.status==ManifestStatus::Applied {applied.push(persisted);}
    }
    if applied.is_empty(){return Ok(());}
    let state = audit.state()?;
    let paths=state.declared_paths.iter().cloned().collect::<Vec<_>>();
    let snapshot=Snapshot::capture(&ctx.setup.canonical_root,&paths,ctx.v2_store)?;
    let before = state.snapshot.as_ref().ok_or_else(|| WorkflowError::StateCorrupt("postapply audit lacks dispatch snapshot".into()))?;
    let old = before.content_index()?;
    let new = snapshot.content_index()?;
    let changed = old.keys().chain(new.keys()).collect::<BTreeSet<_>>().into_iter()
        .filter(|path| old.get(*path) != new.get(*path)).collect::<Vec<_>>();
    let mut unexpected = Vec::new();
    for path in changed {
        if !applied.iter().any(|manifest| patch_accounts_for(manifest, path, &snapshot)) {
            unexpected.push(path.clone());
        }
    }
    audit.update(|state|{
        let records=state.ledger.history.last().map(|r|r.records.clone()).unwrap_or_default();
        for record in records {
            if applied.iter().any(|m|m.changed_files.iter().chain(&m.created_files).chain(&m.deleted_files)
                .any(|p|p==&record.declared_path||record.equivalents.contains(p))) {
                state.ledger.record_applied(&record.declared_path,commit.clone());
            }
        }
        Ok(())
    })?;
    audit.store.with_run_lock(&audit.run_id, |store| store.write_run_json(&audit.run_id,
        &format!("v2/repository-audit/apply-{}-{}.json", sanitize_v2_path_segment(&ctx.execution.call.id), receipt.wave_id),
        &serde_json::json!({"commit":commit,"items_applied":receipt.items_applied,"before":before.identity,
            "after":snapshot.identity,"unexpected_paths":unexpected})))?;
    audit.assess(&snapshot,&paths,if unexpected.is_empty(){"post_apply"}else{"unexpected_change"},ctx.dispatch).await
}

fn patch_accounts_for(manifest: &PatchManifest, path: &str, snapshot: &Snapshot) -> bool {
    if !manifest.changed_files.iter().chain(&manifest.created_files).chain(&manifest.deleted_files).any(|p| p == path) {
        return false;
    }
    if manifest.deleted_files.iter().any(|p| p == path) {
        return !snapshot.root.join(path).exists();
    }
    let Some(expected) = manifest.post_hashes.get(path) else { return false; };
    std::fs::read(snapshot.root.join(path)).is_ok_and(|bytes| blake3::hash(&bytes).to_hex().as_str() == expected)
}
