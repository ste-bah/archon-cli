//! Receiving the bytes of a dropped file.
//!
//! `uploads.rs` answers *whether* an upload is allowed. This module is the only
//! place that takes the bytes, and it re-checks the same policy rather than
//! trusting that the caller asked first — an intent check the client can skip is
//! not a gate.
//!
//! What it deliberately does not do is hand the file to anything. It writes into
//! a staging directory and returns the path. The terminal pane then types
//! `@<path>` into the TUI, which is the TUI's own attachment convention
//! (`event_loop/input.rs`, the file picker injects exactly that), so the browser
//! gains drag-and-drop without the TUI learning anything about the web at all.

use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth, uploads::upload_policy};

/// Longest accepted stored file name, before the extension is re-attached.
/// Long enough for any real document, short enough to stay well inside
/// `MAX_PATH` once joined to a staging directory on Windows.
const MAX_NAME_LEN: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebUploadedFile {
    /// Absolute path on the machine running the server. This is the point of
    /// the endpoint: it is what gets typed into the TUI.
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebUploadResponse {
    pub accepted: bool,
    pub policy_reason: String,
    pub files: Vec<WebUploadedFile>,
}

pub(crate) async fn receive_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let policy = upload_policy(&state.api.policy());
    if !policy.enabled {
        return (
            StatusCode::OK,
            Json(WebUploadResponse {
                accepted: false,
                policy_reason: policy.policy_reason,
                files: Vec::new(),
            }),
        )
            .into_response();
    }

    let root = staging_root(&state.paths.archon_home);
    match store_fields(
        multipart,
        &root,
        policy.max_files,
        policy.max_bytes_per_file,
    )
    .await
    {
        Ok(files) => {
            for file in &files {
                state.live.record("web.upload.stored", &file.file_name);
            }
            (
                StatusCode::OK,
                Json(WebUploadResponse {
                    accepted: true,
                    policy_reason: "upload accepted by web upload policy".to_string(),
                    files,
                }),
            )
                .into_response()
        }
        Err(reason) => (StatusCode::BAD_REQUEST, reason).into_response(),
    }
}

async fn store_fields(
    mut multipart: Multipart,
    root: &Path,
    max_files: u16,
    max_bytes: u64,
) -> Result<Vec<WebUploadedFile>, String> {
    let mut stored = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| format!("upload: malformed multipart body: {error}"))?
    {
        let Some(raw_name) = field.file_name().map(str::to_string) else {
            continue;
        };
        if stored.len() >= max_files as usize {
            return Err(format!(
                "upload: more than {max_files} files in one request"
            ));
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|error| format!("upload: reading '{raw_name}' failed: {error}"))?;
        let size = bytes.len() as u64;
        if size > max_bytes {
            return Err(format!(
                "upload: '{raw_name}' is {size} bytes, over the {max_bytes}-byte limit"
            ));
        }

        let file_name = safe_file_name(&raw_name);
        // One directory per file, named by a fresh uuid. Two drops of the same
        // name must not overwrite each other, and a name that survived
        // sanitisation still must not be able to reach a sibling upload.
        let dir = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("upload: cannot create staging directory: {error}"))?;
        let path = dir.join(&file_name);
        std::fs::write(&path, &bytes)
            .map_err(|error| format!("upload: writing '{file_name}' failed: {error}"))?;

        stored.push(WebUploadedFile {
            path: path.display().to_string(),
            file_name,
            size_bytes: size,
        });
    }

    if stored.is_empty() {
        return Err("upload: no file parts in the request".to_string());
    }
    Ok(stored)
}

/// Reduce a client-supplied name to something safe to join to a directory.
///
/// The name comes from a browser and is attacker-controlled in the case that
/// matters — a page that convinced the operator to drop a file. Taking the
/// final component is not enough on its own, because separators differ by
/// platform and `..` survives basename extraction on some inputs, so every
/// separator and every path-special character is replaced rather than trusted.
fn safe_file_name(raw: &str) -> String {
    let tail = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches('.');
    let cleaned: String = tail
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        return "upload".to_string();
    }
    truncate_keeping_extension(&cleaned)
}

/// Shorten to [`MAX_NAME_LEN`] without losing the extension, which is what
/// every downstream media-type sniff keys off.
fn truncate_keeping_extension(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_LEN {
        return name.to_string();
    }
    let path = Path::new(name);
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name);
    let keep = MAX_NAME_LEN.saturating_sub(ext.chars().count() + 1);
    let stem: String = stem.chars().take(keep.max(1)).collect();
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

fn staging_root(archon_home: &Path) -> PathBuf {
    archon_home.join("web").join("uploads")
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebUploadedFile::decl(&cfg)),
        exported(WebUploadResponse::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
#[path = "uploads_receive_tests.rs"]
mod tests;
