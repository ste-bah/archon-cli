//! Corpus source policy: which paths are readable, how a source is classified
//! for preview, and the raw-bytes endpoint that backs the in-browser PDF
//! viewer.
//!
//! Child module of `corpus` rather than a sibling because everything here is
//! the containment policy for the same corpus roots `corpus.rs` lists and
//! searches; splitting it sideways would let the two drift apart.
//!
//! The bytes endpoint exists because `CorpusSourcePreview.content` is a
//! `String`. A PDF is not a string, and base64 inside the JSON preview would
//! inflate the payload by a third and force the whole document through the
//! query cache. Binary sources get their own response with their own headers.

use std::{
    fs,
    path::{Path, PathBuf},
};

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{AppState, CorpusPreviewQuery, check_auth, source_from_path};

/// Ceiling on a single binary source response.
///
/// The browser holds the whole document in memory before PDF.js parses it, and
/// the server holds it too while writing the response. A corpus PDF larger
/// than this is refused rather than streamed: the viewer cannot usefully page
/// through it anyway.
pub(super) const MAX_BINARY_BYTES: u64 = 32 * 1024 * 1024;

/// How the workbench should render a corpus source.
///
/// Deliberately an enum rather than a `bool`: `previewAvailable` could only
/// ever say "text or nothing", which is the bug this replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum CorpusPreviewMode {
    /// UTF-8 text served inline in `CorpusSourcePreview.content`.
    Text,
    /// PDF fetched as bytes from `/api/corpus/source/bytes` and rendered
    /// page-by-page in the browser.
    Pdf,
    /// A corpus file with no viewer. `policy_reason` names the kind.
    Unsupported,
}

/// Classify a source extension.
///
/// Images are the obvious next binary case and are deliberately absent: an
/// image cannot reach this function today because [`is_corpus_file`] does not
/// admit image extensions, so the corpus never lists one. Adding image preview
/// is an ingest-side change (the extension allow-list plus the Tesseract OCR
/// sidecar text that makes an image searchable in the first place), not a
/// viewer change, so it is left to that work rather than half-built here.
pub(super) fn preview_mode_for(kind: &str) -> CorpusPreviewMode {
    if is_text_preview(kind) {
        CorpusPreviewMode::Text
    } else if kind.eq_ignore_ascii_case("pdf") {
        CorpusPreviewMode::Pdf
    } else {
        CorpusPreviewMode::Unsupported
    }
}

/// `GET /api/corpus/source/bytes` — the raw document behind a binary preview.
pub(crate) async fn bytes_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CorpusPreviewQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match read_binary_source(Path::new(&query.path)) {
        Ok(binary) => binary_response(binary),
        Err(error) => (error.status, error.message).into_response(),
    }
}

pub(super) struct BinarySource {
    pub(super) data: Vec<u8>,
    pub(super) mime: &'static str,
    pub(super) filename: String,
}

pub(super) struct BinaryError {
    pub(super) status: StatusCode,
    pub(super) message: String,
}

fn error(status: StatusCode, message: impl Into<String>) -> BinaryError {
    BinaryError {
        status,
        message: message.into(),
    }
}

pub(super) fn read_binary_source(path: &Path) -> Result<BinarySource, BinaryError> {
    let Some(path) = resolve_corpus_path(path) else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "path is outside configured corpus roots",
        ));
    };
    let Some(source) = source_from_path(&path) else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "path is not a supported corpus file",
        ));
    };
    let mime = match preview_mode_for(&source.kind) {
        CorpusPreviewMode::Pdf => "application/pdf",
        CorpusPreviewMode::Text | CorpusPreviewMode::Unsupported => {
            return Err(error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("{} sources are not served as bytes", source.kind),
            ));
        }
    };
    if source.bytes > MAX_BINARY_BYTES {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "source is {} bytes; the byte endpoint serves at most {MAX_BINARY_BYTES}",
                source.bytes
            ),
        ));
    }
    let data = fs::read(&path).map_err(|err| {
        error(
            StatusCode::NOT_FOUND,
            format!("failed to read source: {err}"),
        )
    })?;
    Ok(BinarySource {
        data,
        mime,
        filename: safe_filename(&source.label),
    })
}

/// Response headers for an untrusted document the user ingested from an
/// arbitrary URL.
///
/// `attachment` and `default-src 'none'; sandbox` matter because this URL can
/// be opened directly in a tab. The workbench itself never navigates to it —
/// it fetches the bytes and hands them to PDF.js — so nothing here is load
/// bearing for the viewer, and all of it is load bearing for anyone who pastes
/// the URL into an address bar.
fn binary_response(binary: BinarySource) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, binary.mime.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", binary.filename),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox".to_string(),
            ),
        ],
        binary.data,
    )
        .into_response()
}

/// Strip anything that could terminate the quoted `filename=` parameter or
/// smuggle a header break. Labels come from the filesystem, not from us.
fn safe_filename(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "source.pdf".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The canonical path of `path` when it resolves inside a corpus root.
///
/// Every corpus read goes through this, and reads the path it returns rather
/// than the one it was given: `..` segments and symlinks are resolved before
/// the containment check, so a symlink inside a root that points outside it
/// is refused, and an allowed link is classified by its real target's
/// extension rather than its own name.
pub(super) fn resolve_corpus_path(path: &Path) -> Option<PathBuf> {
    confine_to_roots(path, &canonical_corpus_roots())
}

/// [`corpus_roots`] that exist, canonicalised once for repeated checks.
pub(super) fn canonical_corpus_roots() -> Vec<PathBuf> {
    corpus_roots()
        .into_iter()
        .filter_map(|(_, root)| root.canonicalize().ok())
        .collect()
}

/// `path` canonicalised, if it lies under one of the already-canonical
/// `roots`. A path that does not exist cannot be canonicalised and is refused.
pub(super) fn confine_to_roots(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let resolved = path.canonicalize().ok()?;
    roots
        .iter()
        .any(|root| resolved.starts_with(root))
        .then_some(resolved)
}

/// Read at most `limit` bytes of a corpus file, re-checking containment.
pub(super) fn read_corpus_file(path: &Path, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let resolved = resolve_corpus_path(path)?;
    let mut bytes = Vec::new();
    fs::File::open(resolved)
        .ok()?
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

pub(super) fn corpus_roots() -> Vec<(String, PathBuf)> {
    let cwd = cwd();
    vec![
        ("repo docs".into(), cwd.join("docs")),
        ("local kb".into(), cwd.join(".archon/kb")),
        ("local docs store".into(), cwd.join(".archon/docs")),
        ("home kb".into(), home_archon().join("kb")),
    ]
}

pub(super) fn is_corpus_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "md" | "txt" | "pdf" | "json" | "jsonl" | "toml" | "yaml" | "yml"
    )
}

pub(super) fn is_text_preview(kind: &str) -> bool {
    matches!(
        kind,
        "md" | "txt" | "json" | "jsonl" | "toml" | "yaml" | "yml"
    )
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_is_the_only_binary_mode_with_a_viewer() {
        assert_eq!(preview_mode_for("md"), CorpusPreviewMode::Text);
        assert_eq!(preview_mode_for("pdf"), CorpusPreviewMode::Pdf);
        assert_eq!(preview_mode_for("PDF"), CorpusPreviewMode::Pdf);
        // Deliberate: no image mode until ingest lists image sources.
        assert_eq!(preview_mode_for("png"), CorpusPreviewMode::Unsupported);
    }

    #[test]
    fn bytes_endpoint_rejects_paths_outside_corpus_roots() {
        let error = read_binary_source(Path::new("/etc/passwd")).err().unwrap();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    /// `<tmp>/root/inside.md`, `<tmp>/outside.md`, `<tmp>/outside_dir/x.md`.
    fn confinement_fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(tmp.path().join("outside_dir")).unwrap();
        fs::write(root.join("inside.md"), "inside").unwrap();
        fs::write(tmp.path().join("outside.md"), "outside secret").unwrap();
        fs::write(tmp.path().join("outside_dir/x.md"), "outside secret").unwrap();
        let root = root.canonicalize().unwrap();
        (tmp, root)
    }

    #[test]
    fn a_file_inside_the_root_is_accepted_as_its_canonical_path() {
        let (_tmp, root) = confinement_fixture();
        let roots = [root.clone()];
        let via_dots = root.join("sub/../inside.md");
        assert_eq!(
            confine_to_roots(&via_dots, &roots),
            Some(root.join("inside.md"))
        );
    }

    #[test]
    fn dot_dot_escapes_are_refused() {
        let (_tmp, root) = confinement_fixture();
        let roots = [root.clone()];
        assert_eq!(confine_to_roots(&root.join("../outside.md"), &roots), None);
        assert_eq!(
            confine_to_roots(&root.join("sub/../../outside.md"), &roots),
            None
        );
    }

    #[test]
    fn absolute_paths_outside_the_root_are_refused() {
        let (tmp, root) = confinement_fixture();
        let roots = [root];
        assert_eq!(
            confine_to_roots(&tmp.path().join("outside.md"), &roots),
            None
        );
        assert_eq!(confine_to_roots(Path::new("/etc/passwd"), &roots), None);
        assert_eq!(confine_to_roots(Path::new("/"), &roots), None);
    }

    #[test]
    fn missing_paths_are_refused() {
        let (_tmp, root) = confinement_fixture();
        assert_eq!(
            confine_to_roots(&root.join("missing.md"), std::slice::from_ref(&root)),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_escaping_the_root_are_refused_and_never_listed() {
        let (tmp, root) = confinement_fixture();
        std::os::unix::fs::symlink(tmp.path().join("outside.md"), root.join("link.md")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside_dir"), root.join("linkdir")).unwrap();
        std::os::unix::fs::symlink(root.join("inside.md"), root.join("alias.md")).unwrap();
        let roots = [root.clone()];

        assert_eq!(confine_to_roots(&root.join("link.md"), &roots), None);
        assert_eq!(confine_to_roots(&root.join("linkdir/x.md"), &roots), None);
        // A link that stays inside the root resolves to its real target.
        assert_eq!(
            confine_to_roots(&root.join("alias.md"), &roots),
            Some(root.join("inside.md"))
        );

        let mut listed = Vec::new();
        super::super::collect_sources(&root, &roots, 0, 50, &mut listed);
        assert!(!listed.is_empty());
        for source in &listed {
            let real = Path::new(&source.path).canonicalize().unwrap();
            assert!(
                real.starts_with(&root),
                "listed a source outside the root: {}",
                source.path
            );
            assert!(
                !matches!(source.label.as_str(), "link.md" | "x.md"),
                "an escaping symlink was listed: {}",
                source.path
            );
        }
    }

    #[test]
    fn filenames_cannot_break_out_of_the_content_disposition_header() {
        assert_eq!(
            safe_filename("re\"port\r\nX-Evil: 1.pdf"),
            "reportX-Evil 1.pdf"
        );
        assert_eq!(safe_filename("../../etc/passwd"), "....etcpasswd");
        assert_eq!(safe_filename("   "), "source.pdf");
    }
}
