//! Which kind of request pressure, if any, an [`LlmError`] reports.

use super::LlmError;

impl LlmError {
    pub fn request_pressure_kind(&self) -> Option<crate::context_window::RequestPressureKind> {
        match self {
            Self::ContextWindowExceeded { .. } => {
                Some(crate::context_window::RequestPressureKind::AggregateContext)
            }
            Self::Http(message) => {
                crate::context_window::classify_request_pressure_error(None, None, None, message)
            }
            Self::Server { status, message } => {
                crate::context_window::classify_request_pressure_error(
                    Some(*status),
                    None,
                    None,
                    message,
                )
            }
            Self::Rejected {
                error_type,
                message,
            } => crate::context_window::classify_request_pressure_error(
                None,
                Some(error_type),
                None,
                message,
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
            Self::Server { status, message } => {
                crate::context_window::classify_context_window_error(
                    Some(*status),
                    None,
                    None,
                    message,
                    None,
                    None,
                )
                .is_some()
            }
            Self::Rejected {
                error_type,
                message,
            } => crate::context_window::classify_context_window_error(
                None,
                Some(error_type),
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
