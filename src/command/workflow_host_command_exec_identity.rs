//! Per-capability reuse keys, with an exact mapping of legacy launch keys.
//!
//! A key names what a call does: its argv, stdin delivery, environment
//! profile, declared writes, remediation scopes and detachment, read under the
//! catalog schema's meaning of them, plus the call's own tokens and stdin.
//! Limits (the timeout and the stdin, stdout and stderr bounds) decide only
//! whether a call is cut short, so they are not in the key: a completed result
//! that no limit cut short stays reusable across a limit change. A result that
//! a limit did cut short is never replayed across one ([`outcome_limits_hold`]).
use super::*;
use archon_workflow::CommandCapability;

/// Whether this build reads a catalog of `schema`. Every schema so far has the
/// same layout; a schema that changes the layout must narrow this.
pub(crate) fn catalog_schema_readable(schema: u32, current: u32) -> bool {
    (1..=current).contains(&schema)
}

/// The schema whose meaning of the non-limit fields a catalog of `schema`
/// has. Every readable schema so far gives argv, stdin, environment, writes
/// and policy the same meaning (a schema bump so far changed only how limits
/// are read), so all map to 1. A bump that changes any of those meanings must
/// map its schemas to a new value here; that re-keys every call it touches.
fn reuse_schema(schema: u32, current: u32) -> WorkflowResult<u32> {
    if catalog_schema_readable(schema, current) {
        Ok(1)
    } else {
        Err(WorkflowError::SpecInvalid(format!(
            "host command catalog schema {schema} is not readable by this build, which reads schemas 1..={current}"
        )))
    }
}

/// What a capability does, without the limits that only cut it short.
fn semantic(capability: &CommandCapability) -> CommandCapability {
    CommandCapability {
        timeout_secs: 0,
        max_stdin_bytes: 0,
        max_stdout_bytes: 0,
        max_stderr_bytes: 0,
        ..capability.clone()
    }
}

/// The limits, and the schema that says how they are read.
fn limits(schema: u32, capability: &CommandCapability) -> [u64; 5] {
    [
        u64::from(schema),
        capability.timeout_secs,
        capability.max_stdin_bytes,
        capability.max_stdout_bytes,
        capability.max_stderr_bytes,
    ]
}

/// A host-command outcome that a limit cut short: timed out, stopped, or with
/// output truncated. Its answer depends on the limits it ran under.
fn cut_short(data: &serde_json::Value) -> bool {
    data["timedOut"] == true
        || data["stdoutTruncated"] == true
        || data["stderrTruncated"] == true
        || matches!(
            data.get("interrupted"),
            Some(serde_json::Value::String(_) | serde_json::Value::Bool(true))
        )
}

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
            Some(launch) => {
                let current = self.catalog.schema_version;
                let schema = reuse_schema(current, current)?;
                let launched = reuse_schema(launch.schema_version, current)?;
                if launched == schema
                    && launch.capabilities.get(&request.command_id).map(semantic)
                        == Some(semantic(capability))
                {
                    // What the launch ran, whatever its limits: keep its key.
                    (
                        launch.digest.clone(),
                        launch.starting_binary_revision.as_str(),
                    )
                } else {
                    // What this capability does changed. Namespace under the
                    // launch revision, so a binary-only upgrade keeps this key.
                    let key = serde_json::json!({
                        "reuse_schema": schema,
                        "capability": semantic(capability),
                    });
                    (
                        archon_workflow::task_set_contract::content_digest(&serde_json::to_vec(
                            &key,
                        )?),
                        launch.starting_binary_revision.as_str(),
                    )
                }
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

    /// Whether a recorded outcome may answer again under this build's limits.
    /// One that no limit cut short does not depend on them. One that a limit
    /// cut short may answer only while the limits (and the schema that reads
    /// them) are the launch's: a pre-upgrade binary ran only the launch
    /// catalog, so that is what every such record was cut by. A record cut by
    /// the limits of a later upgrade that has since been undone is judged by
    /// the launch limits.
    pub(super) fn outcome_limits_hold_for(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowResult<bool> {
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand
            || !cut_short(&record.result.data)
        {
            return Ok(true);
        }
        let Some(launch) = &self.launch_catalog else {
            return Ok(true);
        };
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("persisted HostCommand record has no typed request".into())
        })?;
        Ok(
            match (
                self.catalog.capabilities.get(&request.command_id),
                launch.capabilities.get(&request.command_id),
            ) {
                (Some(current), Some(launched)) => {
                    limits(self.catalog.schema_version, current)
                        == limits(launch.schema_version, launched)
                }
                _ => false,
            },
        )
    }
}
