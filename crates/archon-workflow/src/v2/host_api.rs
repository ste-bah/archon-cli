use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2HostCall {
    pub id: String,
    pub method: WorkflowV2HostMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_mode: Option<WorkflowV2WriteMode>,
    #[serde(default)]
    pub options: WorkflowV2HostOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkflowV2HostOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_kind: Option<String>,
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub target_files_from_item: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<usize>,
    #[serde(default)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowV2HostMethod {
    Agent,
    Fanout,
    Reduce,
    Parallel,
    Tool,
    Implementation,
    Checkpoint,
    QualityGate,
    HumanGate,
    SaveArtifact,
    RequireArtifact,
    FinalReport,
}

impl WorkflowV2HostMethod {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "agent" => Some(Self::Agent),
            "fanout" => Some(Self::Fanout),
            "reduce" => Some(Self::Reduce),
            "parallel" => Some(Self::Parallel),
            "tool" => Some(Self::Tool),
            "implementation" => Some(Self::Implementation),
            "checkpoint" => Some(Self::Checkpoint),
            "qualityGate" => Some(Self::QualityGate),
            "humanGate" => Some(Self::HumanGate),
            "saveArtifact" => Some(Self::SaveArtifact),
            "requireArtifact" => Some(Self::RequireArtifact),
            "finalReport" => Some(Self::FinalReport),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Fanout => "fanout",
            Self::Reduce => "reduce",
            Self::Parallel => "parallel",
            Self::Tool => "tool",
            Self::Implementation => "implementation",
            Self::Checkpoint => "checkpoint",
            Self::QualityGate => "qualityGate",
            Self::HumanGate => "humanGate",
            Self::SaveArtifact => "saveArtifact",
            Self::RequireArtifact => "requireArtifact",
            Self::FinalReport => "finalReport",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2WriteMode {
    Serial,
    Coordinated,
    Worktree,
}

impl WorkflowV2WriteMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "serial" => Some(Self::Serial),
            "coordinated" => Some(Self::Coordinated),
            "worktree" => Some(Self::Worktree),
            _ => None,
        }
    }
}
