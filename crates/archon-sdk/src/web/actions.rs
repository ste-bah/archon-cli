use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{
    AppState,
    api::{EffectivePolicySummary, WebActionDecision},
    check_auth,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebActionRequest {
    pub action_id: String,
    pub action_kind: String,
    pub dry_run: bool,
    pub payload_summary: String,
    pub confirmation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebActionAuditRow {
    pub action_id: String,
    pub action_kind: String,
    pub allowed: bool,
    pub dry_run: bool,
    pub policy_reason: String,
    pub created_at_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebActionResponse {
    pub decision: WebActionDecision,
    pub audit: WebActionAuditRow,
}

pub(crate) async fn evaluate_action_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebActionRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let policy = state.api.policy();
    let response = evaluate_action(request, &policy);
    if let Err(err) = append_audit_row(&response.audit) {
        tracing::warn!("web action audit append failed: {err}");
    }
    (StatusCode::OK, Json(response)).into_response()
}

pub fn evaluate_action(
    request: WebActionRequest,
    policy: &EffectivePolicySummary,
) -> WebActionResponse {
    let decision = decision_for_action(&request, policy);
    let audit = WebActionAuditRow {
        action_id: request.action_id,
        action_kind: request.action_kind,
        allowed: decision.allowed,
        dry_run: request.dry_run,
        policy_reason: decision.policy_reason.clone(),
        created_at_ms: now_ms(),
    };
    WebActionResponse { decision, audit }
}

fn decision_for_action(
    request: &WebActionRequest,
    policy: &EffectivePolicySummary,
) -> WebActionDecision {
    let family = ActionFamily::from_kind(&request.action_kind);
    let requires_confirmation = family.requires_confirmation();
    let dry_run_available = true;

    if !policy.web.allow_mutating_actions {
        return denied(
            requires_confirmation,
            dry_run_available,
            "denied: policy.web.allow_mutating_actions is false",
        );
    }
    if let Some(reason) = family.web_gate_denial(policy) {
        return denied(requires_confirmation, dry_run_available, reason);
    }
    if let Some(reason) = family.subsystem_gate_denial(policy) {
        return denied(requires_confirmation, dry_run_available, reason);
    }
    if requires_confirmation
        && !request.dry_run
        && !request
            .confirmation_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty())
    {
        return denied(
            requires_confirmation,
            dry_run_available,
            "denied: confirmation token is required for this web action",
        );
    }

    WebActionDecision {
        allowed: true,
        requires_confirmation,
        policy_reason: if request.dry_run {
            format!("dry-run allowed by {}", policy.action_gate)
        } else {
            format!("allowed by {}", policy.action_gate)
        },
        dry_run_available,
    }
}

fn denied(
    requires_confirmation: bool,
    dry_run_available: bool,
    reason: impl Into<String>,
) -> WebActionDecision {
    WebActionDecision {
        allowed: false,
        requires_confirmation,
        policy_reason: reason.into(),
        dry_run_available,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionFamily {
    PipelineControl,
    ModelTraining,
    CorpusOpenPath,
    FileUpload,
    BehaviourProposal,
    GenericMutation,
}

impl ActionFamily {
    fn from_kind(action_kind: &str) -> Self {
        let kind = action_kind.trim().to_ascii_lowercase();
        if kind.starts_with("pipeline.") {
            Self::PipelineControl
        } else if kind.starts_with("world.")
            || kind.contains(".candidate.")
            || kind.contains("training")
        {
            Self::ModelTraining
        } else if kind.starts_with("corpus.open") || kind.starts_with("corpus.path.open") {
            Self::CorpusOpenPath
        } else if kind.starts_with("upload.") || kind.starts_with("attachment.") {
            Self::FileUpload
        } else if kind.starts_with("behaviour.") || kind.starts_with("behavior.") {
            Self::BehaviourProposal
        } else {
            Self::GenericMutation
        }
    }

    fn requires_confirmation(self) -> bool {
        matches!(
            self,
            Self::PipelineControl
                | Self::ModelTraining
                | Self::CorpusOpenPath
                | Self::BehaviourProposal
                | Self::GenericMutation
        )
    }

    fn web_gate_denial(self, policy: &EffectivePolicySummary) -> Option<&'static str> {
        match self {
            Self::PipelineControl if !policy.web.allow_pipeline_controls => {
                Some("denied: policy.web.allow_pipeline_controls is false")
            }
            Self::ModelTraining if !policy.web.allow_model_training_actions => {
                Some("denied: policy.web.allow_model_training_actions is false")
            }
            Self::CorpusOpenPath if !policy.web.allow_corpus_open_paths => {
                Some("denied: policy.web.allow_corpus_open_paths is false")
            }
            Self::FileUpload if !policy.web.allow_file_uploads => {
                Some("denied: policy.web.allow_file_uploads is false")
            }
            _ => None,
        }
    }

    fn subsystem_gate_denial(self, policy: &EffectivePolicySummary) -> Option<&'static str> {
        match self {
            Self::PipelineControl if !policy.subsystem.allow_pipeline_controls => {
                Some("denied: pipeline subsystem policy does not allow web controls")
            }
            Self::ModelTraining if !policy.subsystem.allow_model_behavior_changes => {
                Some("denied: policy.world_model.allow_behavior_changes is false")
            }
            Self::CorpusOpenPath if !policy.subsystem.allow_corpus_open_paths => {
                Some("denied: corpus subsystem policy does not allow filesystem open actions")
            }
            Self::FileUpload if !policy.subsystem.allow_file_uploads => {
                Some("denied: upload subsystem policy does not allow file uploads")
            }
            Self::BehaviourProposal if !policy.subsystem.allow_behavior_proposal_actions => {
                Some("denied: learning subsystem policy does not allow behaviour proposal actions")
            }
            _ => None,
        }
    }
}

fn append_audit_row(row: &WebActionAuditRow) -> anyhow::Result<()> {
    let Some(home) = dirs::home_dir() else {
        anyhow::bail!("home directory unavailable");
    };
    let dir = home.join(".archon").join("web");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("actions.audit.jsonl");
    let mut line = serde_json::to_string(row)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))?;
    Ok(())
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebActionRequest::decl(&cfg)),
        exported(WebActionAuditRow::decl(&cfg)),
        exported(WebActionResponse::decl(&cfg)),
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
    fn action_envelope_denies_when_global_gate_is_disabled() {
        let response = evaluate_action(
            WebActionRequest {
                action_id: "act_1".to_string(),
                action_kind: "pipeline.pause".to_string(),
                dry_run: true,
                payload_summary: "pause run".to_string(),
                confirmation_token: None,
            },
            &EffectivePolicySummary::default_safe(),
        );
        assert!(!response.decision.allowed);
        assert!(response.decision.requires_confirmation);
        assert_eq!(response.audit.action_kind, "pipeline.pause");
    }

    #[test]
    fn action_envelope_applies_web_and_subsystem_gates() {
        let mut policy = EffectivePolicySummary::default_safe();
        policy.web.allow_mutating_actions = true;
        policy.web.allow_pipeline_controls = true;
        policy.subsystem.allow_pipeline_controls = true;

        let response = evaluate_action(
            WebActionRequest {
                action_id: "act_2".to_string(),
                action_kind: "pipeline.pause".to_string(),
                dry_run: true,
                payload_summary: "pause run".to_string(),
                confirmation_token: None,
            },
            &policy,
        );

        assert!(response.decision.allowed);
        assert!(response.decision.requires_confirmation);
        assert!(response.decision.policy_reason.contains("dry-run allowed"));
    }

    #[test]
    fn model_actions_require_world_model_subsystem_gate() {
        let mut policy = EffectivePolicySummary::default_safe();
        policy.web.allow_mutating_actions = true;
        policy.web.allow_model_training_actions = true;

        let response = evaluate_action(
            WebActionRequest {
                action_id: "act_3".to_string(),
                action_kind: "world.candidate.promote".to_string(),
                dry_run: true,
                payload_summary: "promote candidate".to_string(),
                confirmation_token: None,
            },
            &policy,
        );

        assert!(!response.decision.allowed);
        assert!(
            response
                .decision
                .policy_reason
                .contains("allow_behavior_changes")
        );
    }
}
