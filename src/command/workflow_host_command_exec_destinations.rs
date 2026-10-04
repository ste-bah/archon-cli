//! Where each declared output of a fixed host command is published.
use super::*;

impl FixedHostCommandExecutor {
    pub(super) fn destinations(
        &self,
        context: &HostCommandResolutionContext,
        command: &ResolvedHostCommand,
        call_id: &str,
    ) -> WorkflowResult<BTreeMap<String, PathBuf>> {
        let envelope = self
            .run_root
            .join("host-command-results")
            .join(call_id)
            .join(ENVELOPE_FILE);
        let mut destinations = BTreeMap::new();
        for path in &command.declared_write_set {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "declared host output {} has no UTF-8 file name",
                        path.display()
                    ))
                })?;
            let destination = match name {
                ENVELOPE_FILE => envelope.clone(),
                "acceptance-contract.json"
                | "acceptance-contract.lock"
                | "task-skeleton.json"
                | "task-skeleton.lock" => context.task_root.join(name),
                "acceptance-pin.json" => crate::command::workflow_task_set::acceptance_pin_path(
                    &context.project_root,
                    &context.task_root,
                ),
                _ if command.command_id == "land-task-body" => {
                    context.frozen_task_file.clone().ok_or_else(|| {
                        WorkflowError::SpecInvalid(
                            "land-task-body has no host-bound frozen task file".to_string(),
                        )
                    })?
                }
                _ => {
                    return Err(WorkflowError::SpecInvalid(format!(
                        "host command '{}' declared unknown output {name}",
                        command.command_id
                    )));
                }
            };
            if destinations.insert(name.to_string(), destination).is_some() {
                return Err(WorkflowError::SpecInvalid(format!(
                    "host command '{}' declared duplicate output {name}",
                    command.command_id
                )));
            }
        }
        Ok(destinations)
    }
}
