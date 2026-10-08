//! Per-capability reuse keys, with an exact mapping of legacy launch keys.
//!
//! A key names what a call does: its argv, stdin delivery, environment
//! profile, declared writes, remediation scopes and detachment, read under the
//! catalog schema's meaning of them, plus the call's own tokens and stdin.
//! Limits (the timeout and the stdin, stdout and stderr bounds) decide only
//! whether a call is cut short, so they are not in the key: a completed result
//! that no limit cut short stays reusable across a limit change. A result that
//! a limit did cut short is replayed only under the limits it recorded
//! ([`FixedHostCommandExecutor::outcome_limits_hold_for`]).
//!
//! Issue 361: the key also names the logic version of the subcommand the
//! capability runs (`workflow_host_command_logic`), so a binary that changes
//! how one capability judges re-keys that capability alone, and every outcome
//! records the version that judged it.
use super::*;
use crate::command::workflow_host_command_logic as logic;
use archon_workflow::CommandCapability;

/// Every catalog schema this build knows, with the schema whose meaning of
/// the non-limit fields (argv, stdin, environment, writes, policy) it carries.
/// A schema that only changes how limits are read keeps its predecessor's
/// meaning, so completed results stay keyed as they were; one that changes any
/// other meaning maps to a new value and re-keys the calls it touches. A
/// schema bump without an entry here fails `upgrade_358_every_built_catalog_schema_is_known`.
pub(crate) const KNOWN_CATALOG_SCHEMAS: &[(u32, u32)] = &[
    (1, 1),
    // Schema 2 reads host timeouts as renewable no-progress windows: limits only.
    (2, 1),
];

/// Whether this build reads a catalog of `schema`: a known schema no newer
/// than the build's own. Every known schema so far has the same layout.
pub(crate) fn catalog_schema_readable(schema: u32, current: u32) -> bool {
    schema <= current
        && KNOWN_CATALOG_SCHEMAS
            .iter()
            .any(|(known, _)| *known == schema)
}

fn reuse_schema(schema: u32, current: u32) -> WorkflowResult<u32> {
    KNOWN_CATALOG_SCHEMAS
        .iter()
        .find(|(known, _)| *known == schema && schema <= current)
        .map(|(_, meaning)| *meaning)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "host command catalog schema {schema} is not readable by this build, which reads the known schemas up to {current}"
            ))
        })
}

/// Where a host command's outcome records the limits it ran under.
pub(crate) const LIMITS_FINGERPRINT: &str = "limitsFingerprint";

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
        let mut tokens = host_command_identity_tokens(context, &request.command_id)?;
        if let Some(version) = logic::key_token(self.logic_version_for(&request.command_id)?) {
            tokens.insert(logic::LOGIC_VERSION_TOKEN.to_string(), version);
        }
        Ok(host_command_call_id(
            &request.command_id,
            &digest,
            revision,
            &tokens,
            request.stdin.as_deref().unwrap_or_default().as_bytes(),
        ))
    }

    /// The logic version capability `id` is judged by in this build.
    pub(super) fn logic_version_for(&self, id: &str) -> WorkflowResult<u32> {
        self.logic
            .get(id)
            .copied()
            .ok_or_else(|| logic::undeclared(id))
    }

    /// A build whose capability `id` runs logic `version`, or declares none
    /// (tests).
    #[cfg(test)]
    pub(crate) fn with_logic_version(mut self, id: &str, version: Option<u32>) -> Self {
        match version {
            Some(version) => self.logic.insert(id.to_string(), version),
            None => self.logic.remove(id),
        };
        self
    }

    /// Another binary, whose build fingerprint is `build` (tests).
    #[cfg(test)]
    pub(crate) fn with_build(mut self, build: &str) -> Self {
        self.build = build.to_string();
        self
    }

    /// Whether a recorded outcome was judged by the logic this build runs
    /// ([`logic::outcome_logic_holds`]). A command this build no longer
    /// declares runs no logic that could vouch for it.
    pub(super) fn outcome_logic_holds_for(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowResult<bool> {
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand {
            return Ok(true);
        }
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("persisted HostCommand record has no typed request".into())
        })?;
        let Some(capability) = self.catalog.capabilities.get(&request.command_id) else {
            return Ok(false);
        };
        let bound = self
            .logic_digests
            .get(&request.command_id)
            .is_some_and(|(_, bound)| *bound)
            .then_some(self.build.as_str());
        let holds = logic::outcome_logic_holds(
            &record.result.data,
            self.logic_version_for(&request.command_id)?,
            logic::judges_only(capability),
            bound,
        );
        if !holds {
            tracing::info!(
                call_id = %record.call.id,
                command_id = %request.command_id,
                recorded = %record.result.data.get(logic::LOGIC_VERSION_STAMP).map_or_else(|| "none".to_string(), ToString::to_string),
                recorded_build = %record.result.data.get(logic::LOGIC_BUILD_STAMP).map_or_else(|| "none".to_string(), ToString::to_string),
                "host command outcome was judged by other logic; it runs again"
            );
        }
        Ok(holds)
    }

    /// The limits a call of `request` runs under here, stamped into its
    /// outcome ([`LIMITS_FINGERPRINT`]).
    pub(super) fn limits_fingerprint_for(
        &self,
        request: &HostCommandRequest,
    ) -> WorkflowResult<Option<serde_json::Value>> {
        Ok(self
            .catalog
            .capabilities
            .get(&request.command_id)
            .map(|capability| serde_json::json!(limits(self.catalog.schema_version, capability))))
    }

    /// Whether a recorded outcome may answer again under this build's limits.
    /// One that no limit cut short does not depend on them. One that a limit
    /// cut short may answer only under the limits it ran under: its own
    /// stamp. An unstamped record was written before outcomes carried one, by
    /// a binary that could run only the launch catalog, so the launch limits
    /// are what cut it short.
    pub(super) fn outcome_limits_hold_for(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowResult<bool> {
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand
            || !cut_short(&record.result.data)
        {
            return Ok(true);
        }
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("persisted HostCommand record has no typed request".into())
        })?;
        let Some(current) = self.limits_fingerprint_for(request)? else {
            return Ok(false);
        };
        Ok(match record.result.data.get(LIMITS_FINGERPRINT) {
            Some(stamp) => *stamp == current,
            None => match &self.launch_catalog {
                None => true,
                Some(launch) => {
                    launch
                        .capabilities
                        .get(&request.command_id)
                        .map(|launched| serde_json::json!(limits(launch.schema_version, launched)))
                        == Some(current)
                }
            },
        })
    }
}
