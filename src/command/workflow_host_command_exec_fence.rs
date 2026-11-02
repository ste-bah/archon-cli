use super::*;
use crate::command::workflow_host_command_operational::{OperationalReport, pause_for_stall};

impl FixedHostCommandExecutor {
    pub(super) fn require_no_stalled_groups(
        &self,
        store: &archon_workflow::WorkflowStore,
        generation: u64,
        report: &OperationalReport<'_>,
        secrets: &HostSecrets,
    ) -> WorkflowResult<()> {
        // A stalled teardown kept its record: processes it started may
        // still write the call's roots, so no retry runs beside them.
        // The run pauses, and a resume refuses until they are gone.
        let groups = crate::command::workflow_host_command_groups::stalled_running(&self.run_root);
        if groups.as_ref().err().is_some_and(|error| {
            error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|source| source.kind() == std::io::ErrorKind::NotADirectory)
            })
        }) {
            return Err(crate::command::workflow_host_fault::workflow(
                groups.err().expect("checked group read error"),
                format!(
                    "host group record directory {} is obstructed; restore this directory before retrying",
                    self.run_root
                        .join(crate::command::workflow_host_command_groups::GROUP_RECORDS_DIR)
                        .display()
                ),
            ));
        }
        let settled = groups
            .map_err(|error| format!("the host command records cannot be read: {error}"))
            .and_then(|records| {
                if records.is_empty() {
                    return Ok(());
                }
                Err(records
                    .iter()
                    .map(|record| {
                        format!(
                            "host command '{}' (process group {}){}",
                            record.command_id,
                            record.pgid,
                            crate::command::workflow_host_command_groups::stall_note(record),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "))
            });
        settled.map_err(|evidence| {
            pause_for_stall(
                store,
                &self.run_root,
                generation,
                report,
                &secrets.text(&evidence),
            )
        })
    }
}
