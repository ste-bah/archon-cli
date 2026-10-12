//! Whether each planned host command would answer from a record: the call id
//! this build's executor gives the planned request, and the record under it.
//! A dry run plans with placeholder ids, so without this it cannot say
//! whether a call replays or runs fresh.

use super::*;
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};

pub(super) struct ReplayProbe {
    executor: FixedHostCommandExecutor,
    results: archon_workflow::WorkflowV2ResultStore,
}

impl ReplayProbe {
    /// An executor bound exactly as the live resume binds it, with no
    /// provider environment: call identity never reads it.
    pub(super) fn new(
        store: &WorkflowStore,
        run_id: &str,
        catalog: archon_workflow::CommandCapabilityCatalog,
        launch_catalog: archon_workflow::CommandCapabilityCatalog,
        (project_root, prd_path, prd_digest, task_root): (&Path, &Path, &str, &Path),
        config: &ArchonConfig,
    ) -> Result<Self> {
        let context = crate::command::workflow_host_command_catalog::HostCommandResolutionContext {
            program: std::env::current_exe().context("resolving the dry-run binary")?,
            project_root: project_root.to_path_buf(),
            prd_path: prd_path.to_path_buf(),
            prd_digest: prd_digest.to_string(),
            task_root: task_root.to_path_buf(),
            run_staging_root: store.run_dir(run_id).join("host-command-staging"),
            frozen_task_id: None,
            frozen_task_file: None,
            freeze_provider_environment: Default::default(),
            acceptance_environment_allowlist: config
                .workflow
                .acceptance_execution
                .as_ref()
                .map(|policy| policy.environment_allowlist.clone())
                .unwrap_or_default(),
            gate_mode: config.workflow.gate_mode,
        };
        Ok(Self {
            executor: FixedHostCommandExecutor::new(catalog, context, store.run_dir(run_id))
                .with_launch_catalog(launch_catalog),
            results: archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        })
    }

    /// ` replay=...` for one planned call. A body landing is not probed: its
    /// identity binds the candidate under the chain's read lock, which a
    /// read-only dry run never takes.
    pub(super) fn note(&self, call: &archon_workflow::WorkflowV2HostCall) -> Result<String> {
        let Some(request) = call.options.host_command.as_ref() else {
            return Ok(String::new());
        };
        if request.command_id == "land-task-body" {
            return Ok(" replay=not-probed".into());
        }
        let id = self.executor.call_identity(request)?;
        Ok(match self.results.load_call_record(&id)? {
            None => format!(" replay=none (fresh call {id})"),
            Some(record) => format!(
                " replay=record {id} status={} logic_holds={}",
                serde_json::to_value(record.status)?
                    .as_str()
                    .unwrap_or_default(),
                self.executor.outcome_logic_holds(&record)?
            ),
        })
    }
}
