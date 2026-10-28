use crate::anthropic::ApiError;
use crate::auth::AuthError;

impl From<AuthError> for crate::provider::LlmError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Provider(error) => error,
            other => Self::Auth(other.to_string()),
        }
    }
}

pub(crate) fn auth_error_to_api(err: AuthError) -> ApiError {
    match err {
        AuthError::Provider(crate::provider::LlmError::Server {
            status,
            message,
            pipeline_retry_after_secs,
        }) => ApiError::ServerError {
            status,
            message,
            retry_after_secs: pipeline_retry_after_secs,
        },
        AuthError::Provider(crate::provider::LlmError::Transport(message)) => {
            ApiError::Transport(message)
        }
        AuthError::Provider(crate::provider::LlmError::Http(message)) => {
            ApiError::HttpError(message)
        }
        AuthError::Provider(crate::provider::LlmError::Rejected {
            error_type,
            message,
        }) => ApiError::Rejected {
            error_type,
            message,
        },
        other => ApiError::AuthError(other.to_string()),
    }
}

pub(crate) fn transport_error(context: &str, error: reqwest::Error) -> AuthError {
    let message = format!("{context}: {error}");
    let fault = crate::provider_fault::from_transport(error);
    if matches!(fault, crate::provider::LlmError::Serialize(_)) {
        AuthError::ParseError(message)
    } else {
        AuthError::Provider(fault)
    }
}
