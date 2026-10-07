//! Secret values a host-command child holds, kept out of what its call records.
//!
//! A child's stdout, stderr and gate envelope are persisted with the call. The
//! child runs with the configured acceptance allowlist and, for provider
//! profiles, the provider credentials, and anything it prints or reports can
//! carry secret values. Credential names and URL credentials select values to replace at this boundary.
//! PATH, HOME and the other process essentials are not secrets and stay as
//! they are; they never enter an identity or digest in the first place.
use std::collections::BTreeMap;
use std::ffi::OsString;

use archon_observability::secret_values::{SecretValues, is_credential_name};
use archon_workflow::{GateEnvelopeV1, WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;

/// Provider settings that are configuration, not credentials.
const PROVIDER_SETTINGS: &[&str] = &["ARCHON_MODEL", "ARCHON_EFFORT", "ARCHON_CONFIG_DIR"];
#[cfg(test)]
pub(crate) use archon_observability::secret_values::REDACTED_VALUE as REDACTED;

pub(crate) fn utf8(bytes: Vec<u8>, stream: &str) -> WorkflowResult<String> {
    String::from_utf8(bytes).map_err(|error| {
        WorkflowError::StageFailed(format!("host command {stream} is not UTF-8: {error}"))
    })
}

pub(crate) struct HostSecrets(SecretValues);

impl HostSecrets {
    /// Provider credentials and credential-named allowlisted values the child was given.
    pub(crate) fn of(
        context: &HostCommandResolutionContext,
        environment: &BTreeMap<String, OsString>,
    ) -> Self {
        let values = context
            .acceptance_environment_allowlist
            .iter()
            .filter(|name| is_credential_name(name))
            .chain(
                context
                    .freeze_provider_environment
                    .keys()
                    .filter(|name| !PROVIDER_SETTINGS.contains(&name.as_str())),
            )
            .filter_map(|name| environment.get(name)?.to_str().map(str::to_string))
            .collect::<Vec<_>>();
        let mut secrets = SecretValues::for_evidence(values.iter().map(String::as_str));
        for name in &context.acceptance_environment_allowlist {
            if let Some(value) = environment.get(name).and_then(|value| value.to_str()) {
                secrets = secrets.with_url_credentials(value);
            }
        }
        Self(secrets)
    }

    pub(crate) fn text(&self, text: &str) -> String {
        let clean = self.0.text(text);
        if self.holds_serialized_secret(clean.as_bytes()) {
            String::new()
        } else {
            clean
        }
    }

    /// Whether `bytes` hold any secret value in clear.
    pub(crate) fn holds_secret(&self, bytes: &[u8]) -> bool {
        self.0.iter().any(|value| {
            let value = value.as_bytes();
            bytes.windows(value.len()).any(|window| window == value)
        })
    }

    /// Verify serialized evidence in raw and JSON-escaped secret spellings.
    pub(crate) fn holds_serialized_secret(&self, bytes: &[u8]) -> bool {
        self.holds_secret(bytes)
            || self.0.iter().any(|value| {
                serde_json::to_string(value).map_or(true, |encoded| {
                    let Some(body) = encoded.get(1..encoded.len().saturating_sub(1)) else {
                        return true;
                    };
                    bytes
                        .windows(body.len())
                        .any(|window| window == body.as_bytes())
                })
            })
    }

    /// Parse child JSON, redacting diagnostics before they leave this boundary.
    pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(
        &self,
        bytes: &[u8],
        context: &str,
    ) -> WorkflowResult<T> {
        serde_json::from_slice(bytes)
            .map_err(|error| WorkflowError::StageFailed(self.text(&format!("{context}: {error}"))))
    }

    /// Redact every free String field, preserving closed wire-contract enums.
    pub(crate) fn envelope(&self, mut envelope: GateEnvelopeV1) -> GateEnvelopeV1 {
        if self.0.is_empty() {
            return envelope;
        }
        self.strings(&mut envelope.report);
        for finding in &mut envelope.policy_findings {
            finding.text = self.text(&finding.text);
            finding.subject = self.text(&finding.subject);
            if let Some(defect) = &mut finding.deterministic_defect {
                defect.code = self.text(&defect.code);
                defect.subject = self.text(&defect.subject);
                defect.location = self.text(&defect.location);
            }
            if let Some(path) = &mut finding.source_path {
                *path = self.text(path);
            }
        }
        if let Some(error) = &mut envelope.operational_error {
            error.text = self.text(&error.text);
            error.kind = self.text(&error.kind);
        }
        envelope
    }

    /// Verify only child-owned data, including serialized scalar values.
    /// Schema keys and closed enum spellings are deliberately excluded.
    pub(crate) fn envelope_holds_secret(&self, envelope: &GateEnvelopeV1) -> bool {
        let mut strings = Vec::new();
        for finding in &envelope.policy_findings {
            strings.extend([&finding.text, &finding.subject]);
            if let Some(defect) = &finding.deterministic_defect {
                strings.extend([&defect.code, &defect.subject, &defect.location]);
            }
            if let Some(path) = &finding.source_path {
                strings.push(path);
            }
        }
        if let Some(error) = &envelope.operational_error {
            strings.extend([&error.text, &error.kind]);
        }
        (envelope.schema_version != archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION
            && self.holds_secret(envelope.schema_version.to_string().as_bytes()))
            || strings
                .into_iter()
                .any(|text| self.holds_secret(text.as_bytes()))
            || serde_json::to_vec(&envelope.report)
                .map_or(true, |bytes| self.holds_serialized_secret(&bytes))
    }

    pub(crate) fn strings(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = self.text(text),
            serde_json::Value::Array(items) => items.iter_mut().for_each(|item| self.strings(item)),
            serde_json::Value::Object(fields) => {
                let original = std::mem::take(fields);
                let reserved: std::collections::BTreeSet<_> =
                    original.keys().map(|key| self.text(key)).collect();
                for (key, mut field) in original {
                    self.strings(&mut field);
                    let clean = self.text(&key);
                    let mut key = clean.clone();
                    let mut suffix = 0u64;
                    while fields.contains_key(&key) {
                        suffix += 1;
                        key = format!("{clean}#{suffix}");
                        while reserved.contains(&key) {
                            suffix += 1;
                            key = format!("{clean}#{suffix}");
                        }
                    }
                    fields.insert(key, field);
                }
            }
            // JSON scalars are child-controlled evidence too. A credential
            // can be a number, boolean or null; changing its type is safer
            // than preserving clear credential bytes in a valid report.
            scalar => {
                if self.holds_serialized_secret(scalar.to_string().as_bytes()) {
                    *scalar = serde_json::Value::String(self.text(&scalar.to_string()));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "workflow_host_secrets_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_host_secrets_regression_tests.rs"]
mod regression_tests;

#[path = "workflow_host_evidence_boundary.rs"]
mod boundary;
pub(crate) use boundary::SealedProcessOutput;
