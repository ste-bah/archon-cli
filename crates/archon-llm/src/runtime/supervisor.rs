use thiserror::Error;

use super::events::{ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity};
use super::status::{ProviderHealthStatus, ProviderRuntimeStatus};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProviderRuntimeSupervisorError {
    #[error("provider event mismatch: expected {expected}, got {actual}")]
    ProviderMismatch { expected: String, actual: String },
}

#[derive(Debug, Clone)]
pub struct ProviderRuntimeSupervisor {
    status: ProviderRuntimeStatus,
    events: Vec<ProviderRuntimeEvent>,
}

impl ProviderRuntimeSupervisor {
    pub fn new(provider_id: impl Into<String>, runtime_mode: impl Into<String>) -> Self {
        Self {
            status: ProviderRuntimeStatus::new(provider_id, runtime_mode),
            events: Vec::new(),
        }
    }

    pub fn from_status(status: ProviderRuntimeStatus) -> Self {
        Self {
            status,
            events: Vec::new(),
        }
    }

    pub fn status(&self) -> &ProviderRuntimeStatus {
        &self.status
    }

    pub fn events(&self) -> &[ProviderRuntimeEvent] {
        &self.events
    }

    pub fn record_event(
        &mut self,
        event: ProviderRuntimeEvent,
    ) -> Result<(), ProviderRuntimeSupervisorError> {
        self.ensure_provider_match(&event)?;
        self.apply_event_to_status(&event);
        self.events.push(event);
        Ok(())
    }

    pub fn request_started(&mut self) -> Result<(), ProviderRuntimeSupervisorError> {
        self.record_event(self.event(
            ProviderRuntimeEventType::RequestStarted,
            ProviderRuntimeSeverity::Debug,
        ))
    }

    pub fn request_retry(
        &mut self,
        retry_count: u32,
        reason_code: impl Into<String>,
    ) -> Result<(), ProviderRuntimeSupervisorError> {
        self.record_event(
            self.event(
                ProviderRuntimeEventType::RequestRetry,
                ProviderRuntimeSeverity::Warn,
            )
            .with_retry_count(retry_count)
            .with_reason(reason_code),
        )
    }

    pub fn request_succeeded(&mut self) -> Result<(), ProviderRuntimeSupervisorError> {
        self.record_event(self.event(
            ProviderRuntimeEventType::RequestSucceeded,
            ProviderRuntimeSeverity::Info,
        ))
    }

    pub fn request_failed(
        &mut self,
        reason_code: impl Into<String>,
        severity: ProviderRuntimeSeverity,
    ) -> Result<(), ProviderRuntimeSupervisorError> {
        self.record_event(
            self.event(ProviderRuntimeEventType::RequestFailed, severity)
                .with_reason(reason_code),
        )
    }

    pub fn rate_limit_observed(
        &mut self,
        reason_code: impl Into<String>,
    ) -> Result<(), ProviderRuntimeSupervisorError> {
        self.record_event(
            self.event(
                ProviderRuntimeEventType::RateLimitObserved,
                ProviderRuntimeSeverity::Warn,
            )
            .with_reason(reason_code),
        )
    }

    fn event(
        &self,
        event_type: ProviderRuntimeEventType,
        severity: ProviderRuntimeSeverity,
    ) -> ProviderRuntimeEvent {
        let mut event = ProviderRuntimeEvent::new(
            self.status.provider_id.clone(),
            self.status.runtime_mode.clone(),
            event_type,
            severity,
        );

        if let Some(profile_id) = &self.status.profile_id {
            event = event.with_profile(profile_id.clone());
        }
        if let Some(model_id) = &self.status.model_id {
            event = event.with_model(model_id.clone());
        }

        event
    }

    fn ensure_provider_match(
        &self,
        event: &ProviderRuntimeEvent,
    ) -> Result<(), ProviderRuntimeSupervisorError> {
        if event.provider_id != self.status.provider_id {
            return Err(ProviderRuntimeSupervisorError::ProviderMismatch {
                expected: self.status.provider_id.clone(),
                actual: event.provider_id.clone(),
            });
        }
        Ok(())
    }

    fn apply_event_to_status(&mut self, event: &ProviderRuntimeEvent) {
        match event.event_type {
            ProviderRuntimeEventType::RequestSucceeded
            | ProviderRuntimeEventType::TokenRefreshed
            | ProviderRuntimeEventType::ProfileCooldownCleared
            | ProviderRuntimeEventType::SpoofIdentitySelected => {
                self.status.health = ProviderHealthStatus::Healthy;
                self.status.last_success_at = Some(event.created_at);
            }
            ProviderRuntimeEventType::RequestFailed
            | ProviderRuntimeEventType::TokenRefreshFailed
            | ProviderRuntimeEventType::FallbackDenied
            | ProviderRuntimeEventType::SpoofIdentityRejected => {
                self.status.health = if event.severity == ProviderRuntimeSeverity::Error {
                    ProviderHealthStatus::Unavailable
                } else {
                    ProviderHealthStatus::Degraded
                };
                self.status.last_failure_at = Some(event.created_at);
            }
            ProviderRuntimeEventType::RateLimitObserved
            | ProviderRuntimeEventType::UsageLimitObserved
            | ProviderRuntimeEventType::ProfileCooldownStarted
            | ProviderRuntimeEventType::FallbackSelected
            | ProviderRuntimeEventType::RequestRetry => {
                self.status.health = ProviderHealthStatus::Degraded;
                self.status.last_failure_at = Some(event.created_at);
            }
            ProviderRuntimeEventType::RequestStarted => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ProviderIdentityStatus;
    use serde_json::json;

    #[test]
    fn supervisor_records_request_lifecycle_and_updates_status() {
        let status = ProviderRuntimeStatus::new("anthropic", "direct")
            .with_profile("oauth-main")
            .with_model("claude-sonnet-4-6")
            .with_identity_status(ProviderIdentityStatus::Spoof);
        let mut supervisor = ProviderRuntimeSupervisor::from_status(status);

        supervisor.request_started().unwrap();
        supervisor.request_succeeded().unwrap();

        assert_eq!(supervisor.events().len(), 2);
        assert_eq!(supervisor.status().health, ProviderHealthStatus::Healthy);
        assert!(supervisor.status().last_success_at.is_some());
        assert_eq!(
            supervisor.events()[0].profile_id.as_deref(),
            Some("oauth-main")
        );
    }

    #[test]
    fn supervisor_rejects_events_for_other_providers() {
        let mut supervisor = ProviderRuntimeSupervisor::new("anthropic", "direct");
        let event = ProviderRuntimeEvent::new(
            "openai",
            "direct",
            ProviderRuntimeEventType::RequestSucceeded,
            ProviderRuntimeSeverity::Info,
        );

        let err = supervisor.record_event(event).unwrap_err();

        assert_eq!(
            err,
            ProviderRuntimeSupervisorError::ProviderMismatch {
                expected: "anthropic".to_string(),
                actual: "openai".to_string(),
            }
        );
        assert!(supervisor.events().is_empty());
    }

    #[test]
    fn rate_limit_event_degrades_status_and_keeps_redacted_metadata() {
        let mut supervisor = ProviderRuntimeSupervisor::new("openai-codex", "auto");
        let event = ProviderRuntimeEvent::new(
            "openai-codex",
            "auto",
            ProviderRuntimeEventType::UsageLimitObserved,
            ProviderRuntimeSeverity::Warn,
        )
        .with_reason("primary_limit")
        .with_redacted_json(json!({"resets_in_minutes": 120}));

        supervisor.record_event(event).unwrap();

        assert_eq!(supervisor.status().health, ProviderHealthStatus::Degraded);
        assert!(supervisor.status().last_failure_at.is_some());
        assert_eq!(
            supervisor.events()[0].raw_redacted_json["resets_in_minutes"],
            120
        );
    }
}
