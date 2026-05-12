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
}

pub(crate) async fn session_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let authenticated = check_auth(&state, &headers).is_ok();
    let session = WebAuthSession {
        authenticated,
        auth_required: state.token.is_some(),
        transport: "bearer-header".to_string(),
        cookie_mode: false,
        csrf_required: false,
    };
    (StatusCode::OK, Json(session)).into_response()
}

pub(crate) async fn logout_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let session = WebAuthSession {
        authenticated: false,
        auth_required: state.token.is_some(),
        transport: "bearer-header".to_string(),
        cookie_mode: false,
        csrf_required: false,
    };
    (StatusCode::OK, Json(session)).into_response()
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
        };
        assert_eq!(session.transport, "bearer-header");
        assert!(!session.cookie_mode);
    }
}
