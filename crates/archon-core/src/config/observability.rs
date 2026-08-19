//! `[observability]` — where telemetry goes (#189 Phase 10).
//!
//! Its own section rather than a field on `[logging]` because the two answer
//! different questions. `[logging]` is about a file on this machine;
//! `otlp_endpoint` sends session telemetry *off* it, which is a different
//! decision with a different blast radius, and burying it under a heading about
//! log rotation would hide that.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ObservabilityConfig {
    /// OTLP/HTTP traces endpoint, e.g. `http://127.0.0.1:4318/v1/traces`.
    ///
    /// `None` — the default — means no exporter is built and no per-span state
    /// is allocated. Setting it sends span names and their attributes to that
    /// collector. Attributes are redacted first, by the same layer that scrubs
    /// the session log, but the destination is still off this machine: point it
    /// somewhere you control.
    pub otlp_endpoint: Option<String>,
}

impl ObservabilityConfig {
    /// The configured endpoint, if it is more than whitespace.
    ///
    /// An empty string in the file means "off", the same as the key being
    /// absent — commenting a value out by blanking it is the obvious thing to
    /// try, and it should not produce an exporter aimed at nowhere.
    #[must_use]
    pub fn endpoint(&self) -> Option<&str> {
        self.otlp_endpoint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_is_off_by_default() {
        assert_eq!(ObservabilityConfig::default().endpoint(), None);
    }

    #[test]
    fn a_blank_endpoint_reads_as_off_rather_than_as_nowhere() {
        let blanked = ObservabilityConfig {
            otlp_endpoint: Some("   ".to_string()),
        };

        assert_eq!(blanked.endpoint(), None);
    }

    #[test]
    fn a_configured_endpoint_is_returned_trimmed() {
        let configured = ObservabilityConfig {
            otlp_endpoint: Some("  http://127.0.0.1:4318/v1/traces ".to_string()),
        };

        assert_eq!(
            configured.endpoint(),
            Some("http://127.0.0.1:4318/v1/traces")
        );
    }
}
