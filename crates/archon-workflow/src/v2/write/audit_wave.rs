//! Post-apply audit credit is derived from persisted apply manifests, never claims.
use super::*;
use crate::repository_audit::runtime::Snapshot;

pub(super) async fn after_apply(ctx:&WorktreePlanRunContext<'_>,artifacts:&WorktreeWaveArtifacts)->WorkflowResult<()> {
    let Some(audit)=ctx.dispatch.repository_audit() else{return Ok(());};
    let mut applied=Vec::new();
    for manifest in &artifacts.manifests {
        let path=PathBuf::from(manifest_path_for(&ctx.setup.run_root,&ctx.execution.call.id,&manifest.item_id));
        let persisted:PatchManifest=serde_json::from_slice(&std::fs::read(&path).map_err(|e|WorkflowError::io(&path,e))?)?;
        if persisted.status==ManifestStatus::Applied {applied.push(persisted);}
    }
    if applied.is_empty(){return Ok(());}
    let commit=crate::write_coordinator::worktree_isolation::run_git(&["rev-parse","HEAD"],&ctx.setup.canonical_root)
        .map_err(|e|WorkflowError::StageFailed(e.to_string()))?;
    let commit=String::from_utf8_lossy(&commit.stdout).trim().to_string();
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
    let paths=audit.state()?.declared_paths.into_iter().collect::<Vec<_>>();
    let snapshot=Snapshot::capture(&ctx.setup.canonical_root,&paths,ctx.v2_store)?;
    audit.assess(&snapshot,&paths,"post_apply",ctx.dispatch).await
}
