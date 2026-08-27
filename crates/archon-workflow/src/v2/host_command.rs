//! Provider-neutral contracts for trusted host command capabilities.
//!
//! A workflow script selects only a symbolic capability and bounded stdin.
//! Executable, argv, cwd, environment, limits, destinations, write sets, and
//! reuse identity remain host-owned.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{WorkflowError, WorkflowResult};

const HOST_COMMAND_IDENTITY_DOMAIN: &[u8] = b"host-command-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCommandRequest {
    pub command_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}

impl HostCommandRequest {
    /// Absolute bridge bound. Catalog capabilities may impose a smaller limit.
    pub const MAX_STDIN_BYTES: usize = 2 * 1024 * 1024;

    pub fn new(command_id: impl Into<String>, stdin: Option<String>) -> WorkflowResult<Self> {
        let command_id = command_id.into().trim().to_string();
        if command_id.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "hostCommand requires a non-empty command capability id".to_string(),
            ));
        }
        if stdin
            .as_ref()
            .is_some_and(|value| value.len() > Self::MAX_STDIN_BYTES)
        {
            return Err(WorkflowError::SpecInvalid(format!(
                "hostCommand stdin exceeds the {} byte bridge limit",
                Self::MAX_STDIN_BYTES
            )));
        }
        Ok(Self { command_id, stdin })
    }
}

/// Stable identity for one resolved host command invocation.
///
/// Every component is length-framed. This avoids ambiguous concatenations such
/// as `(a, bc)` and `(ab, c)` while preserving exact stdin bytes.
pub fn host_command_call_id(
    command_id: &str,
    catalog_digest: &str,
    starting_binary_revision: &str,
    resolved_tokens: &BTreeMap<String, String>,
    stdin: &[u8],
) -> String {
    let mut framed = Vec::new();
    push_frame(&mut framed, HOST_COMMAND_IDENTITY_DOMAIN);
    push_frame(&mut framed, command_id.as_bytes());
    push_frame(&mut framed, catalog_digest.as_bytes());
    push_frame(&mut framed, starting_binary_revision.as_bytes());
    push_frame(&mut framed, &(resolved_tokens.len() as u64).to_le_bytes());
    for (key, value) in resolved_tokens {
        push_frame(&mut framed, key.as_bytes());
        push_frame(&mut framed, value.as_bytes());
    }
    push_frame(&mut framed, stdin);
    blake3::hash(&framed).to_hex().to_string()
}

fn push_frame(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StdinDelivery {
    None,
    Utf8Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentProfileId {
    None,
    FreezeProvider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCapability {
    pub id: String,
    pub argv_template: Vec<String>,
    pub stdin_delivery: StdinDelivery,
    pub environment_profile: EnvironmentProfileId,
    pub timeout_secs: u64,
    pub max_stdin_bytes: u64,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub declared_write_set: Vec<String>,
    pub remediation_scopes: std::collections::BTreeSet<super::gate_envelope::RemediationScope>,
    pub detaches: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCapabilityCatalog {
    pub schema_version: u32,
    pub starting_binary_revision: String,
    pub digest: String,
    pub capabilities: BTreeMap<String, CommandCapability>,
}

impl CommandCapabilityCatalog {
    pub fn recompute_digest(&mut self) -> WorkflowResult<()> {
        let mut canonical = self.clone();
        canonical.digest.clear();
        self.digest = blake3::hash(&serde_json::to_vec(&canonical)?)
            .to_hex()
            .to_string();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCommandSubject {
    pub task_id: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPostconditionEvaluation {
    pub satisfied: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCommandResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub timed_out: bool,
    pub interrupted: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_envelope: Option<super::gate_envelope::GateEnvelopeV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_receipt: Option<super::publication::PublicationReceiptV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<HostCommandSubject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postcondition: Option<CommandPostconditionEvaluation>,
}

impl HostCommandResult {
    pub fn reusable(&self) -> bool {
        self.exit_code == Some(0)
            && self.publication_receipt.is_some()
            && self
                .gate_envelope
                .as_ref()
                .is_some_and(|envelope| envelope.operational_error.is_none())
            && self
                .postcondition
                .as_ref()
                .is_some_and(|postcondition| postcondition.satisfied)
            && !self.timed_out
            && !self.interrupted
            && !self.stdout_truncated
            && !self.stderr_truncated
    }
}
