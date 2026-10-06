//! Per-capability reuse, with an exact mapping of legacy launch keys.
use super::*;

impl FixedHostCommandExecutor {
    pub(crate) fn with_launch_catalog(mut self, launch: CommandCapabilityCatalog) -> Self {
        self.launch_catalog = Some(launch);
        self
    }

    pub(super) fn content_identity(
        &self,
        request: &HostCommandRequest,
        context: &HostCommandResolutionContext,
    ) -> WorkflowResult<String> {
        let capability = self
            .catalog
            .capabilities
            .get(&request.command_id)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "undeclared host command capability '{}'",
                    request.command_id
                ))
            })?;
        let (digest, revision) = match &self.launch_catalog {
            Some(launch) if launch.capabilities.get(&request.command_id) == Some(capability) => {
                // Exactly the argv, stdin delivery, environment profile, limits,
                // writes and policy that the launch ran: preserve its v1 key.
                (
                    launch.digest.clone(),
                    launch.starting_binary_revision.as_str(),
                )
            }
            Some(launch) => {
                // Only this capability changed. Namespace under the launch
                // revision, so another binary-only upgrade preserves this key.
                (
                    archon_workflow::task_set_contract::content_digest(&serde_json::to_vec(
                        capability,
                    )?),
                    launch.starting_binary_revision.as_str(),
                )
            }
            None => (
                self.catalog.digest.clone(),
                self.catalog.starting_binary_revision.as_str(),
            ),
        };
        Ok(host_command_call_id(
            &request.command_id,
            &digest,
            revision,
            &host_command_identity_tokens(context, &request.command_id)?,
            request.stdin.as_deref().unwrap_or_default().as_bytes(),
        ))
    }
}
