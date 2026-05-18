use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebAuthSession {
    pub authenticated: bool,
    pub auth_required: bool,
    pub transport: String,
    pub cookie_mode: bool,
    pub csrf_required: bool,
    pub server_side_logout_supported: bool,
    pub logout_message: String,
}

pub(crate) async fn session_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let authenticated = check_auth(&state, &headers).is_ok();
    let session = web_auth_session(authenticated, state.token.is_some());
    (StatusCode::OK, Json(session)).into_response()
}

pub(crate) async fn logout_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let session = web_auth_session(true, state.token.is_some());
    (StatusCode::OK, Json(session)).into_response()
}

fn web_auth_session(authenticated: bool, auth_required: bool) -> WebAuthSession {
    WebAuthSession {
        authenticated,
        auth_required,
        transport: "bearer-header".to_string(),
        cookie_mode: false,
        csrf_required: false,
        server_side_logout_supported: false,
        logout_message: if auth_required {
            "Bearer-token auth has no server-side session to invalidate; clients must discard the token."
                .to_string()
        } else {
            "No bearer token is required for this local web session.".to_string()
        },
    }
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    exported(WebAuthSession::decl(&cfg)) + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_session_shape_defaults_to_header_transport() {
        let session = WebAuthSession {
            authenticated: true,
            auth_required: true,
            transport: "bearer-header".to_string(),
            cookie_mode: false,
            csrf_required: false,
            server_side_logout_supported: false,
            logout_message: "Bearer-token auth has no server-side session to invalidate; clients must discard the token.".to_string(),
        };
        assert_eq!(session.transport, "bearer-header");
        assert!(!session.cookie_mode);
    }

    #[test]
    fn bearer_logout_shape_is_honest_about_server_side_invalidation() {
        let session = web_auth_session(true, true);
        assert!(session.authenticated);
        assert!(!session.server_side_logout_supported);
        assert!(session.logout_message.contains("discard the token"));
    }
}
