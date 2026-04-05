/// TASK-CLI-414: Web UI Mode tests.
///
/// Run with:  cargo test -p archon-sdk -- web --test-threads=1
use archon_sdk::web::{WebConfig, WebServer};

// ---------------------------------------------------------------------------
// WebConfig defaults
// ---------------------------------------------------------------------------

#[test]
fn web_config_default_port() {
    assert_eq!(WebConfig::default().port, 8421);
}

#[test]
fn web_config_default_bind_address() {
    assert_eq!(WebConfig::default().bind_address, "127.0.0.1");
}

#[test]
fn web_config_default_open_browser_true() {
    assert!(WebConfig::default().open_browser);
}

#[test]
fn web_config_is_localhost_true_for_loopback() {
    let cfg = WebConfig::default();
    assert!(cfg.is_localhost());
}

#[test]
fn web_config_is_localhost_false_for_any() {
    let cfg = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        ..WebConfig::default()
    };
    assert!(!cfg.is_localhost());
}

// ---------------------------------------------------------------------------
// Static asset embedding
// ---------------------------------------------------------------------------

#[test]
fn embedded_index_html_exists() {
    let html = archon_sdk::web::assets::get_asset("index.html");
    assert!(html.is_some(), "index.html must be embedded");
}

#[test]
fn embedded_index_html_contains_doctype() {
    let asset = archon_sdk::web::assets::get_asset("index.html").unwrap();
    let text = std::str::from_utf8(asset.data.as_ref()).unwrap();
    assert!(
        text.contains("<!DOCTYPE html>") || text.contains("<!doctype html>"),
        "index.html must be a valid HTML document"
    );
}

#[test]
fn embedded_styles_css_exists() {
    let css = archon_sdk::web::assets::get_asset("styles.css");
    assert!(css.is_some(), "styles.css must be embedded");
}

#[test]
fn list_embedded_assets_non_empty() {
    let files = archon_sdk::web::assets::list_assets();
    assert!(!files.is_empty(), "embedded asset list must not be empty");
}

// ---------------------------------------------------------------------------
// WebServer construction
// ---------------------------------------------------------------------------

#[test]
fn web_server_new_with_default_config() {
    let cfg = WebConfig::default();
    // Should not panic when constructing
    let _server = WebServer::new(cfg, None);
}

#[test]
fn web_server_new_with_token() {
    let cfg = WebConfig::default();
    let _server = WebServer::new(cfg, Some("mytoken".to_string()));
}

// ---------------------------------------------------------------------------
// Mime-type resolution
// ---------------------------------------------------------------------------

#[test]
fn mime_type_html() {
    assert_eq!(
        archon_sdk::web::assets::mime_type("index.html"),
        "text/html; charset=utf-8"
    );
}

#[test]
fn mime_type_css() {
    assert_eq!(
        archon_sdk::web::assets::mime_type("styles.css"),
        "text/css; charset=utf-8"
    );
}

#[test]
fn mime_type_js() {
    assert_eq!(
        archon_sdk::web::assets::mime_type("app.js"),
        "application/javascript; charset=utf-8"
    );
}

#[test]
fn mime_type_unknown_falls_back_to_octet_stream() {
    assert_eq!(
        archon_sdk::web::assets::mime_type("file.xyz"),
        "application/octet-stream"
    );
}
