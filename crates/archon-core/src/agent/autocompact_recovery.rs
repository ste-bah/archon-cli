use super::*;

pub(super) const MAX_COMPACT_FAILURES: u32 = 3;
const DEFAULT_TRANSIENT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

pub use archon_llm::context_window::RequestPressureKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryTier {
    FullCompaction,
    EmergencyProjection,
}

#[derive(Debug, Clone, Default)]
pub struct RecoveryLadder {
    attempts: u8,
}

impl RecoveryLadder {
    pub fn next(&mut self, _classification: RequestPressureKind) -> Option<RecoveryTier> {
        let tier = match self.attempts {
            0 => RecoveryTier::FullCompaction,
            1 => RecoveryTier::EmergencyProjection,
            _ => return None,
        };
        self.attempts += 1;
        Some(tier)
    }

    pub fn attempts(&self) -> u8 {
        self.attempts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryTelemetry {
    pub classification: RequestPressureKind,
    pub tier: RecoveryTier,
    pub before_body_bytes: usize,
    pub after_body_bytes: usize,
    pub before_estimated_tokens: u64,
    pub after_estimated_tokens: u64,
    pub reduced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_secs: Option<u64>,
}

impl RecoveryTelemetry {
    pub fn new(
        classification: RequestPressureKind,
        tier: RecoveryTier,
        before_body_bytes: usize,
        after_body_bytes: usize,
    ) -> Self {
        Self {
            classification,
            tier,
            before_body_bytes,
            after_body_bytes,
            before_estimated_tokens: approx_tokens_from_bytes(before_body_bytes),
            after_estimated_tokens: approx_tokens_from_bytes(after_body_bytes),
            reduced: after_body_bytes < before_body_bytes,
            cooldown_secs: None,
        }
    }

    pub fn with_cooldown_secs(mut self, cooldown_secs: Option<u64>) -> Self {
        self.cooldown_secs = cooldown_secs;
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct AutoCompactState {
    pub compaction_count: u32,
    pub consecutive_failures: u32,
    pub transient_failures: u32,
    pub structural_failures: u32,
    pub cooldown_until: Option<std::time::Instant>,
    pub disabled: bool,
    pub compact_in_flight: bool,
    pub last_compact_at_tokens: u64,
}

impl AutoCompactState {
    pub fn should_attempt(&self) -> bool {
        !self.disabled
            && !self.compact_in_flight
            && self
                .cooldown_until
                .is_none_or(|deadline| std::time::Instant::now() >= deadline)
    }

    pub fn on_success(&mut self, tokens: u64) {
        self.compaction_count += 1;
        self.consecutive_failures = 0;
        self.transient_failures = 0;
        self.structural_failures = 0;
        self.cooldown_until = None;
        self.compact_in_flight = false;
        self.last_compact_at_tokens = tokens;
    }

    pub fn on_ordinary_success(&mut self) {
        self.transient_failures = 0;
        self.cooldown_until = None;
    }

    pub fn on_failure(&mut self, error: &CompactionError) {
        self.compact_in_flight = false;
        match compaction_failure_disposition(error) {
            CompactionFailureDisposition::Cancelled => {}
            CompactionFailureDisposition::Transient { cooldown } => {
                self.transient_failures += 1;
                self.consecutive_failures = self.transient_failures;
                self.cooldown_until = Some(std::time::Instant::now() + cooldown);
            }
            CompactionFailureDisposition::NoSafeBoundary => {
                self.consecutive_failures = 0;
            }
            CompactionFailureDisposition::Structural => {
                self.structural_failures += 1;
                self.consecutive_failures = self.structural_failures;
                if self.structural_failures >= MAX_COMPACT_FAILURES {
                    self.disabled = true;
                }
            }
        }
    }

    pub fn on_cancel(&mut self) {
        self.compact_in_flight = false;
    }

    pub fn cooldown_remaining_secs(&self) -> Option<u64> {
        self.cooldown_until.map(|deadline| {
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_secs()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionFailureDisposition {
    Cancelled,
    Transient { cooldown: std::time::Duration },
    NoSafeBoundary,
    Structural,
}

pub fn compaction_failure_disposition(error: &CompactionError) -> CompactionFailureDisposition {
    match error {
        CompactionError::Cancelled => CompactionFailureDisposition::Cancelled,
        CompactionError::Provider(archon_llm::provider::LlmError::RateLimited {
            retry_after_secs,
        }) => CompactionFailureDisposition::Transient {
            cooldown: std::time::Duration::from_secs(*retry_after_secs),
        },
        CompactionError::Provider(archon_llm::provider::LlmError::Overloaded)
        | CompactionError::Provider(archon_llm::provider::LlmError::Server {
            status: 500..=599,
            ..
        }) => CompactionFailureDisposition::Transient {
            cooldown: DEFAULT_TRANSIENT_COOLDOWN,
        },
        CompactionError::NoSafeBoundary => CompactionFailureDisposition::NoSafeBoundary,
        CompactionError::InvalidSummary(_) => CompactionFailureDisposition::Structural,
        CompactionError::Provider(_) => CompactionFailureDisposition::Structural,
    }
}

pub fn request_pressure_kind_for_request(
    error: &archon_llm::provider::LlmError,
    request: &archon_llm::provider::LlmRequest,
) -> Option<RequestPressureKind> {
    let classification = error.request_pressure_kind()?;
    if classification == RequestPressureKind::AggregateContext
        && request.messages.len() == 1
        && request.messages[0]
            .get("role")
            .and_then(serde_json::Value::as_str)
            == Some("user")
        && !message_has_tool_result(&request.messages[0])
    {
        return Some(RequestPressureKind::OpeningPrompt);
    }
    Some(classification)
}

fn message_has_tool_result(message: &serde_json::Value) -> bool {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|block| {
                block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
            })
        })
}

pub fn classify_stream_error(
    provider: &str,
    error_type: &str,
    message: &str,
) -> archon_llm::provider::LlmError {
    archon_llm::context_window::classify_context_window_error(
        None,
        Some(error_type),
        None,
        message,
        Some(provider),
        None,
    )
    .unwrap_or_else(|| archon_llm::provider::LlmError::Http(format!("{error_type}: {message}")))
}
