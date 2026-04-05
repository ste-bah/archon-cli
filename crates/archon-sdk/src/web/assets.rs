/// Static asset serving via `rust-embed`.
///
/// The `web/dist/` directory (relative to the workspace root) is compiled
/// into the binary at build time. At runtime, assets are served directly
/// from memory without touching the filesystem.
use rust_embed::{EmbeddedFile, RustEmbed};

/// Embedded static files from `web/dist/`.
///
/// The path is relative to the archon-sdk crate directory (`crates/archon-sdk/`).
/// `../../web/dist` resolves to `project-work/archon-cli/web/dist/`.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
pub struct WebAssets;

/// A borrowed asset with its MIME type.
pub struct Asset {
    /// Raw bytes of the file.
    pub data: std::borrow::Cow<'static, [u8]>,
    /// MIME type string (e.g. `"text/html; charset=utf-8"`).
    pub mime: &'static str,
}

/// Return the embedded asset for `path`, or `None` if not found.
pub fn get_asset(path: &str) -> Option<Asset> {
    let f: EmbeddedFile = <WebAssets as RustEmbed>::get(path)?;
    Some(Asset {
        mime: mime_type(path),
        data: f.data,
    })
}

/// Return all embedded asset file paths.
pub fn list_assets() -> Vec<String> {
    <WebAssets as RustEmbed>::iter()
        .map(|s| s.into_owned())
        .collect()
}

/// Resolve the MIME type for a file path by extension.
pub fn mime_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}
