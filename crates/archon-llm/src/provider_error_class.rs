//! Which kind of request pressure, if any, an [`LlmError`] reports.
//!
//! Text classifiers here read exactly the input they read at 86fbab995: the
//! stored field text, and that commit's Display text. None of these methods
//! returns that text.
use super::LlmError;

impl LlmError {
    // A frozen copy of 86fbab995's Display, template for template: each field
    // that Display passed through `diagnostic_message` is redacted here too,
    // and ProviderNotFound prints its fields as stored, as it did then. It
    // can differ from today's (fully redacted) Display only for that variant.
    // Private: it feeds the classifiers below and is never returned.
    fn main_display(&self) -> String {
        use crate::provider_fault::diagnostic_message as redacted;
        let suffix = |message: &super::DiagnosticMessage| {
            if message.raw().is_empty() {
                String::new()
            } else {
                format!(": {}", redacted(message.raw()))
            }
        };
        match self {
            Self::Http(message) | Self::Transport(message) => {
                format!("HTTP error: {}", redacted(message.raw()))
            }
            Self::Auth(message) => format!("authentication error: {}", redacted(message.raw())),
            Self::RateLimited {
                message,
                retry_after_secs,
                ..
            } => format!(
                "rate limited: retry after {retry_after_secs}s{}",
                suffix(message)
            ),
            Self::Rejected {
                error_type,
                message,
            } => format!(
                "provider rejected the request ({}): {}",
                redacted(error_type.raw()),
                redacted(message.raw())
            ),
            Self::Overloaded { message, .. } => format!("server overloaded{}", suffix(message)),
            // The server message was a redacted DiagnosticMessage at 86fbab995.
            Self::Server {
                status, message, ..
            } => format!("server error ({status}): {}", message.as_str()),
            Self::Serialize(message) => format!("serialization error: {}", redacted(message.raw())),
            Self::Unsupported(message) => {
                format!("feature not supported: {}", redacted(message.raw()))
            }
            Self::ProviderNotFound { name, available } => format!(
                "provider '{}' not found — available: [{}]",
                name.raw(),
                available.raw()
            ),
            Self::QuotaExceeded(message) => format!("quota exceeded: {}", redacted(message.raw())),
            Self::Aborted => "request aborted".to_string(),
            Self::ContextWindowExceeded {
                provider_message, ..
            } => format!(
                "context window exceeded: {}",
                redacted(provider_message.raw())
            ),
        }
    }

    /// This error's 86fbab995 Display as a diagnostic: that text for
    /// classifiers, today's redacted Display for egress. Forwarders that
    /// used `to_string()` then use this now.
    pub fn display_message(&self) -> super::DiagnosticMessage {
        super::DiagnosticMessage::from_raw_and_text(self.main_display(), &self.to_string())
    }

    // `outer` (a Display text showing this fault: its own, or a wrapper's
    // that embeds it) as 86fbab995 saw it: today's Display of this fault
    // reads as that commit's Display. Private: only the closed operations
    // below read it, and only their plain results leave this crate.
    fn shown_text(&self, outer: &str) -> String {
        let shown = self.to_string();
        let main = self.main_display();
        if shown == main {
            outer.to_string()
        } else {
            outer.replace(&shown, &main)
        }
    }

    /// Run one closed text test over `outer` as 86fbab995 saw it.
    pub fn shown_is(&self, outer: &str, test: ShownTest) -> bool {
        test.matches(&self.shown_text(outer))
    }

    /// The retry log category of `outer` as 86fbab995 saw it.
    pub fn shown_log_class(&self, outer: &str) -> &'static str {
        crate::provider_fault::retry_error_class_text(&self.shown_text(outer))
    }

    /// The pipeline status scan of `outer` as 86fbab995 saw it.
    pub fn shown_status(&self, outer: &str) -> (bool, u16) {
        crate::provider_fault::http_status_pattern(&self.shown_text(outer))
    }

    /// The provider fault one chain element carries: the element itself, or
    /// the fault a transparent wrapper holds (its `source()` skips the fault).
    pub fn carried_by<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a LlmError> {
        error.downcast_ref::<LlmError>().or_else(|| {
            match error.downcast_ref::<crate::auth::AuthError>()? {
                crate::auth::AuthError::Provider(fault) => Some(fault),
                _ => None,
            }
        })
    }

    /// The first provider fault in a source chain, transparent wrappers included.
    pub fn in_chain<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a LlmError> {
        std::iter::successors(Some(error), |error| error.source()).find_map(Self::carried_by)
    }

    /// Whether 86fbab995's Display text contains this status code.
    pub fn mentions_status(&self, code: u16) -> bool {
        self.main_display().contains(&code.to_string())
    }

    pub fn request_pressure_kind(&self) -> Option<crate::context_window::RequestPressureKind> {
        match self {
            Self::ContextWindowExceeded { .. } => {
                Some(crate::context_window::RequestPressureKind::AggregateContext)
            }
            Self::Http(message) => crate::context_window::classify_request_pressure_error(
                None,
                None,
                None,
                message.raw(),
            ),
            Self::Server {
                status, message, ..
            } => crate::context_window::classify_request_pressure_error(
                Some(*status),
                None,
                None,
                message.as_str(),
            ),
            Self::Rejected {
                error_type,
                message,
            } => crate::context_window::classify_request_pressure_error(
                None,
                Some(error_type.raw()),
                None,
                message.raw(),
            ),
            _ => None,
        }
    }

    pub fn is_context_window_exceeded(&self) -> bool {
        match self {
            Self::ContextWindowExceeded { .. } => true,
            Self::Http(message) => crate::context_window::classify_context_window_error(
                None, None, None, message, None, None,
            )
            .is_some(),
            Self::Server {
                status, message, ..
            } => crate::context_window::classify_context_window_error(
                Some(*status),
                None,
                None,
                message.as_str(),
                None,
                None,
            )
            .is_some(),
            Self::Rejected {
                error_type,
                message,
            } => crate::context_window::classify_context_window_error(
                None,
                Some(error_type.raw()),
                None,
                message,
                None,
                None,
            )
            .is_some(),
            _ => false,
        }
    }
}

/// A closed set of text tests. Each runs inside this crate over a text that
/// may show a provider fault; only the boolean leaves. The rule bodies are
/// the single sources the owning crates call for their own text as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShownTest {
    /// The text contains this fixed marker.
    Contains(&'static str),
    /// The ASCII-lowercased text contains this fixed marker.
    ContainsLowercase(&'static str),
    /// The workflow send-message rule after its no-progress guard.
    SendRetry,
    /// The fanout retry fragments.
    FanoutRetry,
    /// A context-window phrase (no status known).
    ContextWindow,
    /// The pipeline transient-attempt rule.
    PipelineTransient,
    /// The pipeline overload error codes.
    PipelineOverload,
}

impl ShownTest {
    /// Apply this test to a text the caller already holds.
    pub fn matches(self, text: &str) -> bool {
        match self {
            Self::Contains(marker) => text.contains(marker),
            Self::ContainsLowercase(marker) => text.to_ascii_lowercase().contains(marker),
            Self::SendRetry => crate::provider_fault::send_retry_text(text),
            Self::FanoutRetry => crate::provider_fault::fanout_retry_text_matches(text),
            Self::ContextWindow => {
                crate::context_window::context_window_text_matches(None, None, None, text)
            }
            Self::PipelineTransient => crate::provider_fault::pipeline_transient_text(text),
            Self::PipelineOverload => {
                crate::provider_fault::pipeline_overload_code(&text.to_ascii_lowercase())
            }
        }
    }
}
