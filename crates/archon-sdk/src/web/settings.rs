use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

const DEFAULT_ACCENT: &str = "mint";
const DEFAULT_ACCENT_HEX: &str = "#87d8b4";
const DEFAULT_ACCENT_STRONG_HEX: &str = "#2fbc86";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebThemeProfile {
    pub theme_mode: String,
    pub density_mode: String,
    pub accent_id: String,
    pub accent_hex: String,
    pub accent_strong_hex: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebThemeProfileEnvelope {
    pub profile: WebThemeProfile,
    pub storage_path: String,
    pub persisted: bool,
    pub export_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebThemeProfileSaveRequest {
    pub profile: WebThemeProfile,
}

pub(crate) async fn theme_profile_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    (StatusCode::OK, Json(read_theme_profile())).into_response()
}

pub(crate) async fn save_theme_profile_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebThemeProfileSaveRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let profile = sanitize_profile(request.profile);
    match write_theme_profile(&profile) {
        Ok(envelope) => (StatusCode::OK, Json(envelope)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("theme profile save failed: {error}"),
        )
            .into_response(),
    }
}

fn read_theme_profile() -> WebThemeProfileEnvelope {
    let path = theme_profile_path();
    let persisted = path.exists();
    let profile = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<WebThemeProfile>(&text).ok())
        .map(sanitize_profile)
        .unwrap_or_else(default_profile);
    envelope(path, profile, persisted)
}

fn write_theme_profile(profile: &WebThemeProfile) -> anyhow::Result<WebThemeProfileEnvelope> {
    let path = theme_profile_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let profile = sanitize_profile(profile.clone());
    fs::write(&path, serde_json::to_string_pretty(&profile)?)?;
    Ok(envelope(path, profile, true))
}

fn envelope(path: PathBuf, profile: WebThemeProfile, persisted: bool) -> WebThemeProfileEnvelope {
    let export_json = serde_json::to_string_pretty(&profile).unwrap_or_else(|_| "{}".into());
    WebThemeProfileEnvelope {
        profile,
        storage_path: display_path(&path),
        persisted,
        export_json,
    }
}

fn sanitize_profile(mut profile: WebThemeProfile) -> WebThemeProfile {
    if !matches!(profile.theme_mode.as_str(), "dark" | "light") {
        profile.theme_mode = "dark".into();
    }
    if !matches!(profile.density_mode.as_str(), "comfortable" | "compact") {
        profile.density_mode = "comfortable".into();
    }
    if !matches!(
        profile.accent_id.as_str(),
        "mint" | "blue" | "amber" | "rose"
    ) {
        profile.accent_id = DEFAULT_ACCENT.into();
        profile.accent_hex = DEFAULT_ACCENT_HEX.into();
        profile.accent_strong_hex = DEFAULT_ACCENT_STRONG_HEX.into();
    }
    if !valid_hex(&profile.accent_hex) {
        profile.accent_hex = DEFAULT_ACCENT_HEX.into();
    }
    if !valid_hex(&profile.accent_strong_hex) {
        profile.accent_strong_hex = DEFAULT_ACCENT_STRONG_HEX.into();
    }
    if profile.updated_at_ms == 0 {
        profile.updated_at_ms = now_ms();
    }
    profile
}

fn default_profile() -> WebThemeProfile {
    WebThemeProfile {
        theme_mode: "dark".into(),
        density_mode: "comfortable".into(),
        accent_id: DEFAULT_ACCENT.into(),
        accent_hex: DEFAULT_ACCENT_HEX.into(),
        accent_strong_hex: DEFAULT_ACCENT_STRONG_HEX.into(),
        updated_at_ms: now_ms(),
    }
}

fn valid_hex(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit())
}

fn theme_profile_path() -> PathBuf {
    home_archon().join("web/theme-profile.json")
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebThemeProfile::decl(&cfg)),
        exported(WebThemeProfileEnvelope::decl(&cfg)),
        exported(WebThemeProfileSaveRequest::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_profile_bounds_import_values() {
        let profile = sanitize_profile(WebThemeProfile {
            theme_mode: "solarized".into(),
            density_mode: "tiny".into(),
            accent_id: "purple".into(),
            accent_hex: "red".into(),
            accent_strong_hex: "#not-ok".into(),
            updated_at_ms: 0,
        });
        assert_eq!(profile.theme_mode, "dark");
        assert_eq!(profile.density_mode, "comfortable");
        assert_eq!(profile.accent_id, "mint");
        assert_eq!(profile.accent_hex, DEFAULT_ACCENT_HEX);
    }
}
