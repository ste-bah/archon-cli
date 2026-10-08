//! Durable launch intent before a child can execute any code.
use super::super::workflow_host_command_groups::{GroupRecordGuard, record_group};
use archon_workflow::{WorkflowError, WorkflowResult};
use std::path::Path;

pub(super) struct LaunchBarrier(Option<GroupRecordGuard>);
impl LaunchBarrier {
    pub(super) fn reserve(dir: Option<&Path>, command: &str) -> WorkflowResult<Self> {
        // Different executors use different host pids; local owner claims
        // distinguish overlapping launches in this process without a cross-
        // process collision on a shared zero-pid filename.
        let record = dir
            .map(|dir| record_group(dir, std::process::id(), 0, None, None, command))
            .transpose()?;
        Ok(Self(record))
    }
    /// Only after no child was launched, confirmed teardown, or a durable
    /// incomplete command record now protects the launched child.
    pub(super) fn clear(&mut self) -> WorkflowResult<()> {
        if let Some(record) = self.0.take() {
            let paths = record.paths();
            drop(record);
            for path in paths {
                match std::fs::symlink_metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    other => {
                        return Err(WorkflowError::HostOperational(format!(
                            "cannot clear launch barrier {} ({other:?}); restore write permission and resume again",
                            path.display()
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}
impl Drop for LaunchBarrier {
    fn drop(&mut self) {
        if let Some(record) = self.0.take() {
            record.keep(None);
        }
    }
}
