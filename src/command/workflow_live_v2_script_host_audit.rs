//! All call-cache routes share audit admission before granting completion credit.
use super::*;
use archon_workflow::repository_audit::{reuse, runtime::Snapshot};

impl WorkflowScriptHost {
    fn audit_cache_paths(record: &WorkflowV2CallRecord) -> Vec<String> {
        let mut paths = record.call.options.target_files.clone();
        paths.extend(record.result.files_changed.iter().map(|file| file.path.clone()));
        paths.sort();
        paths.dedup();
        paths
    }

    pub(super) fn audit_cache_eligible(&self, record: &WorkflowV2CallRecord)
        -> archon_workflow::WorkflowResult<bool>
    {
        let persisted = reuse::load_state(&self.runner.v2_store)?;
        if record.call.write_mode.is_none() { return Ok(true); }
        let state = match &self.runner.client.audit {
            Some(audit) => Some(audit.state()?),
            None => persisted,
        };
        let Some(state) = state else { return Ok(true); };
        // Dynamic fanouts re-enter the branch planner, which knows the complete
        // current declaration set and can reuse eligible siblings individually.
        if record.call.options.target_files_from_item { return Ok(false); }
        reuse::admits(&state, &Self::audit_cache_paths(record))
    }

    pub(super) async fn refresh_audit_for_cache(&self, record: &WorkflowV2CallRecord)
        -> archon_workflow::WorkflowResult<bool>
    {
        if record.call.write_mode.is_none() { return self.audit_cache_eligible(record); }
        if record.call.options.target_files_from_item { return Ok(false); }
        if let Some(audit) = &self.runner.client.audit {
            let _boundary = audit.lock_write_boundary().await;
            if let Some(root) = &self.runner.runtime.target_repository_root {
                let paths = Self::audit_cache_paths(record);
                let mut all = audit.state()?.declared_paths;
                all.extend(paths);
                let all = all.into_iter().collect::<Vec<_>>();
                let snapshot = Snapshot::capture(std::path::Path::new(root), &all, &self.runner.v2_store)?;
                audit.assess(&snapshot, &all, "cache", &AuditDispatch(self.runner.client.for_audit())).await?;
            }
        }
        self.audit_cache_eligible(record)
    }
}
