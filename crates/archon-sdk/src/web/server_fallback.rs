//! What an unrouted path answers.
//!
//! Split from server.rs under the 500-line gate. It sits beside the router
//! rather than inside it because the judgement it makes is about the shape of
//! a URL -- page path or API path -- not about serving one.

use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};

/// Send an unrouted page path to its hash form instead of answering 404.
///
/// The workbench is hash-routed, so `/ingest` is genuinely not a route this
/// server has -- but it is what a person types, bookmarks, or reaches by
/// refreshing. A 404 there reads as "that page does not exist", which is the
/// wrong answer about a page that does.
///
/// API and asset paths are deliberately exempt. A mistyped endpoint must stay
/// a 404: a client that followed a redirect into HTML would report a parse
/// failure rather than the missing route it actually asked for, which is the
/// harder bug to find. Non-GET methods are 404 for the same reason -- a POST
/// to an unknown path is a caller error, not a navigation.
pub(super) async fn spa_fallback_handler(method: Method, uri: Uri) -> Response {
    let path = uri.path();
    if method != Method::GET
        || path.starts_with("/api/")
        || path.starts_with("/static/")
        || path.starts_with("/health")
    {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let view = path.trim_matches('/');
    if view.is_empty() {
        return Redirect::temporary("/").into_response();
    }

    // The query survives the hop so a link carrying parameters still arrives
    // with them; the SPA reads its own query from after the fragment.
    let target = match uri.query() {
        Some(query) if !query.is_empty() => format!("/#/{view}?{query}"),
        _ => format!("/#/{view}"),
    };
    Redirect::temporary(&target).into_response()
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    fn uri(value: &str) -> Uri {
        value.parse().expect("uri")
    }

    fn location(response: &Response) -> Option<String> {
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }

    /// The reported defect: typing or refreshing a workbench URL answered 404
    /// for a page that exists.
    #[tokio::test]
    async fn a_typed_page_path_is_sent_to_its_hash_route() {
        let response = spa_fallback_handler(Method::GET, uri("/ingest")).await;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(location(&response).as_deref(), Some("/#/ingest"));
    }

    #[tokio::test]
    async fn a_query_string_survives_the_hop() {
        let response = spa_fallback_handler(Method::GET, uri("/corpus?doc=abc")).await;

        assert_eq!(location(&response).as_deref(), Some("/#/corpus?doc=abc"));
    }

    /// A mistyped endpoint must stay a 404. Redirecting it into HTML would make
    /// a client report a JSON parse failure instead of the missing route it
    /// actually asked for.
    #[tokio::test]
    async fn an_unknown_api_path_is_still_not_found() {
        let response = spa_fallback_handler(Method::GET, uri("/api/does-not-exist")).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(location(&response).is_none());
    }

    #[tokio::test]
    async fn a_missing_asset_is_still_not_found() {
        let response = spa_fallback_handler(Method::GET, uri("/static/gone.js")).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A POST to an unknown path is a caller error, not navigation.
    #[tokio::test]
    async fn a_non_get_to_an_unknown_path_is_not_redirected() {
        let response = spa_fallback_handler(Method::POST, uri("/ingest")).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
