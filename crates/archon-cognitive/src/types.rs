use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SituationKind {
    Greeting,
    HighRisk,
    GitMutation,
    CiDebug,
    PipelineControl,
    WorldModelTask,
    CodeChange,
    Research,
    SimpleQuestion,
    Ambiguous,
}

impl SituationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greeting => "greeting",
            Self::HighRisk => "high_risk",
            Self::GitMutation => "git_mutation",
            Self::CiDebug => "ci_debug",
            Self::PipelineControl => "pipeline_control",
            Self::WorldModelTask => "world_model_task",
            Self::CodeChange => "code_change",
            Self::Research => "research",
            Self::SimpleQuestion => "simple_question",
            Self::Ambiguous => "ambiguous",
        }
    }

    pub fn is_trivial(self) -> bool {
        matches!(self, Self::Greeting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveSurface {
    Cli,
    Tui,
    Web,
    Pipeline,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierConfidence {
    High,
    Medium,
    Low,
}

impl ClassifierConfidence {
    pub fn from_score(score: f32) -> Self {
        if score >= 0.85 {
            Self::High
        } else if score >= 0.55 {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Situation {
    pub id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub user_text_hash: String,
    pub kind: SituationKind,
    pub confidence_score: f32,
    pub confidence: ClassifierConfidence,
    pub reason: String,
    pub surface: CognitiveSurface,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolVerdict {
    Allow { reason: String },
    Suppress { reason: String },
    ConvertToContextNote { note: String },
}

impl ToolVerdict {
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Allow { reason } | Self::Suppress { reason } => reason,
            Self::ConvertToContextNote { note } => note,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveDecision {
    pub id: String,
    pub situation_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub tool_name: Option<String>,
    pub verdict: ToolVerdict,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum CognitiveError {
    #[error("cognitive io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cognitive schema error: {0}")]
    Schema(String),
    #[error("cognitive store error: {0}")]
    Store(String),
    #[error("cognitive serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub fn hash_user_text(user_text: &str) -> String {
    blake3::hash(user_text.as_bytes()).to_hex().to_string()
}

pub fn direct_response_for(kind: SituationKind) -> Option<&'static str> {
    match kind {
        SituationKind::Greeting => Some("Hello. I'm here."),
        _ => None,
    }
}
