use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, WebConfig, WebRuntimePaths, assets, check_auth};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ApiStatus {
    pub status: String,
    pub version: String,
    pub web: WebRuntimeStatus,
    pub features: WebFeatureSummary,
    pub stores: Vec<WebStoreStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebRuntimeStatus {
    pub bind_address: String,
    pub port: u16,
    pub auth_required: bool,
    pub max_body_bytes: usize,
    pub dev_mode: bool,
    pub asset_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebFeatureSummary {
    pub chat: bool,
    pub uploads: bool,
    pub memory_learning: bool,
    pub world_model: bool,
    pub reasoning_quality: bool,
    pub corpus: bool,
    pub pipelines: bool,
    pub metrics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebStoreStatus {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EffectiveConfigSummary {
    pub web: WebConfigSummary,
    pub frontend_stack: FrontendStackSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebConfigSummary {
    pub port: u16,
    pub bind_address: String,
    pub open_browser: bool,
    pub max_body_bytes: usize,
    pub auth_required: bool,
    pub non_loopback_bind: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct FrontendStackSummary {
    pub framework: String,
    pub bundler: String,
    pub generated_types: bool,
    pub asset_delivery: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EffectivePolicySummary {
    pub web: WebPolicySummary,
    pub subsystem: WebSubsystemPolicySummary,
    pub action_gate: String,
    pub requires_confirmation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebPolicySummary {
    pub allow_mutating_actions: bool,
    pub allow_file_uploads: bool,
    pub allow_pipeline_controls: bool,
    pub allow_model_training_actions: bool,
    pub allow_corpus_open_paths: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebSubsystemPolicySummary {
    pub allow_behavior_proposal_actions: bool,
    pub allow_model_behavior_changes: bool,
    pub allow_pipeline_controls: bool,
    pub allow_corpus_open_paths: bool,
    pub allow_file_uploads: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebActionDecision {
    pub allowed: bool,
    pub requires_confirmation: bool,
    pub policy_reason: String,
    pub dry_run_available: bool,
}

#[derive(Debug, Clone)]
pub struct WebApiState {
    status: ApiStatus,
    config: EffectiveConfigSummary,
    policy: EffectivePolicySummary,
}

impl WebApiState {
    pub fn from_server_config(
        config: &WebConfig,
        auth_required: bool,
        policy: EffectivePolicySummary,
        paths: WebRuntimePaths,
    ) -> Self {
        let dev_mode = std::env::var("ARCHON_WEB_DEV").ok().as_deref() == Some("1");
        let status = ApiStatus {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            web: WebRuntimeStatus {
                bind_address: config.bind_address.clone(),
                port: config.port,
                auth_required,
                max_body_bytes: config.max_body_bytes,
                dev_mode,
                asset_mode: if dev_mode { "vite-dev" } else { "embedded" }.to_string(),
            },
            features: WebFeatureSummary::foundation(&policy),
            stores: WebStoreStatus::known_stores(&paths),
        };

        let config_summary = EffectiveConfigSummary {
            web: WebConfigSummary {
                port: config.port,
                bind_address: config.bind_address.clone(),
                open_browser: config.open_browser,
                max_body_bytes: config.max_body_bytes,
                auth_required,
                non_loopback_bind: !config.is_localhost(),
            },
            frontend_stack: FrontendStackSummary {
                framework: "React 19".to_string(),
                bundler: "Vite".to_string(),
                generated_types: true,
                asset_delivery: "embedded web/dist assets".to_string(),
            },
        };

        Self {
            status,
            config: config_summary,
            policy,
        }
    }

    pub fn status(&self) -> ApiStatus {
        self.status.clone()
    }

    pub fn config(&self) -> EffectiveConfigSummary {
        self.config.clone()
    }

    pub fn policy(&self) -> EffectivePolicySummary {
        self.policy.clone()
    }
}

impl WebFeatureSummary {
    fn foundation(policy: &EffectivePolicySummary) -> Self {
        Self {
            chat: true,
            uploads: policy.web.allow_mutating_actions
                && policy.web.allow_file_uploads
                && policy.subsystem.allow_file_uploads,
            memory_learning: true,
            world_model: true,
            reasoning_quality: true,
            corpus: true,
            pipelines: true,
            metrics: true,
        }
    }
}

impl WebStoreStatus {
    fn known_stores(paths: &WebRuntimePaths) -> Vec<Self> {
        vec![
            Self::from_path("memory db", paths.memory_db.clone()),
            Self::from_path("session activity", paths.session_activity_root.clone()),
            Self::from_path("world-model", paths.world_model_root.clone()),
            Self::from_path("corpus", paths.cwd.join(".archon/docs")),
            Self::from_path("pipelines", paths.archon_home.join("pipelines")),
            Self::embedded_assets(),
        ]
    }

    fn from_path(name: &str, path: std::path::PathBuf) -> Self {
        let exists = path.exists();
        Self {
            name: name.to_string(),
            status: if exists { "ready" } else { "missing" }.to_string(),
            detail: path.to_string_lossy().to_string(),
        }
    }

    fn embedded_assets() -> Self {
        let files = assets::list_assets().len();
        Self {
            name: "web assets".to_string(),
            status: if files > 0 { "ready" } else { "missing" }.to_string(),
            detail: format!("embedded web/dist assets ({files} files)"),
        }
    }
}

impl WebPolicySummary {
    pub fn default_safe() -> Self {
        Self {
            allow_mutating_actions: false,
            allow_file_uploads: false,
            allow_pipeline_controls: false,
            allow_model_training_actions: false,
            allow_corpus_open_paths: false,
        }
    }
}

impl WebSubsystemPolicySummary {
    pub fn default_safe() -> Self {
        Self {
            allow_behavior_proposal_actions: false,
            allow_model_behavior_changes: false,
            allow_pipeline_controls: false,
            allow_corpus_open_paths: false,
            allow_file_uploads: false,
        }
    }
}

impl EffectivePolicySummary {
    pub fn default_safe() -> Self {
        Self {
            web: WebPolicySummary::default_safe(),
            subsystem: WebSubsystemPolicySummary::default_safe(),
            action_gate: "global web mutation gate AND action-family gate AND subsystem gate"
                .to_string(),
            requires_confirmation: vec![
                "pipeline control".to_string(),
                "model promotion".to_string(),
                "training action".to_string(),
                "corpus filesystem open".to_string(),
                "behaviour proposal approval".to_string(),
            ],
        }
    }
}

pub(crate) async fn status_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    authed_json(&state, &headers, state.api.status())
}

pub(crate) async fn config_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    authed_json(&state, &headers, state.api.config())
}

pub(crate) async fn policy_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    authed_json(&state, &headers, state.api.policy())
}

fn authed_json<T: Serialize>(state: &AppState, headers: &HeaderMap, value: T) -> Response {
    if let Err(resp) = check_auth(state, headers) {
        return resp;
    }
    (StatusCode::OK, Json(value)).into_response()
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(ApiStatus::decl(&cfg)),
        exported(WebRuntimeStatus::decl(&cfg)),
        exported(WebFeatureSummary::decl(&cfg)),
        exported(WebStoreStatus::decl(&cfg)),
        exported(EffectiveConfigSummary::decl(&cfg)),
        exported(WebConfigSummary::decl(&cfg)),
        exported(FrontendStackSummary::decl(&cfg)),
        exported(EffectivePolicySummary::decl(&cfg)),
        exported(WebPolicySummary::decl(&cfg)),
        exported(WebSubsystemPolicySummary::decl(&cfg)),
        exported(WebActionDecision::decl(&cfg)),
    ]
    .into_iter()
    .chain([
        super::live::generated_typescript(),
        super::actions::generated_typescript(),
        super::auth::generated_typescript(),
        super::chat::generated_typescript(),
        super::uploads::generated_typescript(),
        super::cognitive::generated_typescript(),
        super::corpus::generated_typescript(),
        super::ingest::generated_typescript(),
        super::inspect::generated_typescript(),
        super::metrics::generated_typescript(),
        super::pipelines::generated_typescript(),
        super::settings::generated_typescript(),
        super::world::generated_typescript(),
        super::evidence::generated_typescript(),
    ])
    .collect::<Vec<_>>()
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    #[test]
    fn default_policy_is_inspection_only() {
        let policy = WebPolicySummary::default_safe();
        assert!(!policy.allow_mutating_actions);
        assert!(!policy.allow_model_training_actions);
    }

    #[test]
    fn status_reflects_server_config() {
        let cfg = WebConfig {
            port: 9001,
            bind_address: "0.0.0.0".to_string(),
            open_browser: false,
            max_body_bytes: 1024,
        };
        let state = WebApiState::from_server_config(
            &cfg,
            true,
            EffectivePolicySummary::default_safe(),
            WebRuntimePaths::default(),
        );
        assert_eq!(state.status().web.port, 9001);
        assert_eq!(state.status().web.max_body_bytes, 1024);
        assert!(state.config().web.non_loopback_bind);
        assert_eq!(state.config().web.max_body_bytes, 1024);
        assert!(state.status().web.auth_required);
    }

    #[test]
    fn generated_web_api_types_match_checked_in() {
        let generated = generated_typescript();
        let path = generated_types_path();
        if std::env::var("UPDATE_WEB_TYPES").ok().as_deref() == Some("1") {
            fs::write(&path, &generated).expect("write generated web API types");
        }
        let checked_in = fs::read_to_string(&path).expect("read checked-in web API types");
        assert_eq!(
            checked_in, generated,
            "web TypeScript DTOs are stale; run UPDATE_WEB_TYPES=1 cargo test -p archon-sdk generated_web_api_types_match_checked_in"
        );
    }

    fn generated_types_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/src/api/generated/web.ts")
            .canonicalize()
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api/generated/web.ts")
            })
    }
}
