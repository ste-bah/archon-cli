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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateActionKind {
    AnswerDirectly,
    RecallMemory,
    InspectFiles,
    SearchDocs,
    RunSafeShellProbe,
    AskClarification,
    RunTests,
    DeferOrDecline,
    CreateGovernedProposal,
    RunLearningTick,
}

impl CandidateActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AnswerDirectly => "answer_directly",
            Self::RecallMemory => "recall_memory",
            Self::InspectFiles => "inspect_files",
            Self::SearchDocs => "search_docs",
            Self::RunSafeShellProbe => "safe_shell_probe",
            Self::AskClarification => "ask_clarification",
            Self::RunTests => "run_tests",
            Self::DeferOrDecline => "defer_decline",
            Self::CreateGovernedProposal => "create_governed_proposal",
            Self::RunLearningTick => "run_learning_tick",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "answer_directly" => Self::AnswerDirectly,
            "recall_memory" => Self::RecallMemory,
            "inspect_files" => Self::InspectFiles,
            "search_docs" => Self::SearchDocs,
            "safe_shell_probe" => Self::RunSafeShellProbe,
            "ask_clarification" => Self::AskClarification,
            "run_tests" => Self::RunTests,
            "defer_decline" => Self::DeferOrDecline,
            "create_governed_proposal" => Self::CreateGovernedProposal,
            "run_learning_tick" => Self::RunLearningTick,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreSource {
    Heuristic,
    JepaTransition,
    LatentTransition,
    PredictionUnavailable,
}

impl ScoreSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::JepaTransition => "jepa_transition",
            Self::LatentTransition => "latent_transition",
            Self::PredictionUnavailable => "prediction_unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub situation_id: String,
    pub action_kind: CandidateActionKind,
    pub tool_name: Option<String>,
    pub expected_evidence: String,
    pub expected_user_output: String,
    pub risk_class: RiskLevel,
    pub rollback_path: Option<String>,
    pub heuristic_score: f32,
    pub score_source: ScoreSource,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub candidate_id: String,
    pub action_kind: CandidateActionKind,
    pub rejection_reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateScore {
    pub candidate_id: String,
    pub score: f32,
    pub score_source: ScoreSource,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub situation_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub selected_candidate_id: String,
    pub rejected_alternatives: Vec<RejectedCandidate>,
    pub heuristic_scores: Vec<CandidateScore>,
    pub policy_verdict: Option<String>,
    pub verification_contract: Option<String>,
    pub user_visible_summary: String,
    pub created_at: DateTime<Utc>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutiveStateSnapshot {
    pub session_id: String,
    pub turn_number: u64,
    pub stage: String,
    pub situation_id: String,
    pub user_text_hash: String,
    pub situation_kind: SituationKind,
    pub selected_candidate_id: Option<String>,
    pub selected_action: Option<CandidateActionKind>,
    pub policy_summary: String,
    pub verification_summary: String,
    pub prediction_available: bool,
    pub reflection_id: Option<String>,
    pub degraded: Vec<String>,
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
