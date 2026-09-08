//! Materialize a private content-addressed audit view from captured wave source.
use std::{collections::BTreeMap, path::Path};
use crate::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};
use crate::write_coordinator::{WritePlan, WriteCoordinatorConfig};
use crate::write_coordinator::worktree_isolation::{SealedSource, capture_sealed_source, run_git};
use super::runtime::Snapshot;

fn error(e: impl std::fmt::Display) -> WorkflowError { WorkflowError::StageFailed(format!("repository audit snapshot: {e}")) }
impl Snapshot {
    pub fn from_sealed(root: &Path, source: &SealedSource, plan: &WritePlan, store: &WorkflowV2ResultStore) -> WorkflowResult<Self> {
        let mut plan = plan.clone();
        plan.isolated_root=store.root().join("repository-audit/snapshots").join(uuid::Uuid::new_v4().to_string());
        plan.item_id="repository-audit".into();
        let view=source.assessment_workspace(root,&plan).map_err(error)?;
        let identity=String::from_utf8_lossy(&run_git(&["rev-parse","HEAD^{tree}"],&view.plan.isolated_root).map_err(error)?.stdout).trim().to_string();
        let listing=run_git(&["ls-files","-z"],&view.plan.isolated_root).map_err(error)?.stdout;
        let paths=listing.split(|b|*b==0).filter(|p|!p.is_empty()).map(|p|String::from_utf8(p.to_vec()).map_err(error)).collect::<WorkflowResult<Vec<_>>>()?;
        Ok(Self{identity,root:view.plan.isolated_root,paths})
    }
    pub fn capture(root:&Path,paths:&[String],store:&WorkflowV2ResultStore)->WorkflowResult<Self>{
        let plan=snapshot_plan(root,paths,store)?;
        let source=capture_sealed_source(root,&plan,&WriteCoordinatorConfig::default()).map_err(error)?;
        Self::from_sealed(root,&source,&plan,store)
    }
    pub fn content_index(&self)->WorkflowResult<BTreeMap<String,String>>{
        let listing=run_git(&["ls-tree","-r","-z","HEAD"],&self.root).map_err(error)?.stdout;
        let mut index=BTreeMap::new();
        for row in listing.split(|b|*b==0).filter(|r|!r.is_empty()){
            let row=std::str::from_utf8(row).map_err(error)?;
            let (meta,path)=row.split_once('\t').ok_or_else(||error("invalid tree entry"))?;
            index.insert(path.into(),meta.into());
        }
        Ok(index)
    }
}
pub fn snapshot_plan(root:&Path,paths:&[String],store:&WorkflowV2ResultStore)->WorkflowResult<WritePlan>{
    use crate::write_coordinator::write_plan::{normalize_target,TargetFilesSource};
    Ok(WritePlan{run_id:"repository-audit".into(),stage_id:"repository-audit".into(),item_id:"repository-audit".into(),
        canonical_root:root.into(),isolated_root:store.root().join("repository-audit/capture"),
        target_files:paths.iter().map(|p|normalize_target(p,root).map_err(error)).collect::<WorkflowResult<Vec<_>>>()?,
        target_dir_scopes:vec![],target_files_source:TargetFilesSource::Item,read_context_files:vec![],verify_inputs:vec![],
        baseline_id:"sealed".into(),workspace_boundary_required:true,resource_keys:Default::default()})
}
