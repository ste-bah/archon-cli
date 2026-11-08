use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeFloorFacts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collection_findings: Vec<String>,
    pub artifact_present: bool,
    pub artifact_byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_json: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_json: Option<Value>,
    #[serde(default)]
    pub instance_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub searched_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, FileFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<InstanceFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_binding: Option<InstanceBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileFact {
    pub shown: String,
    pub state: FileState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FileState {
    Absent,
    Empty,
    Bytes(Vec<u8>),
    DirectoryEvidence(u64),
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceFact {
    pub reference: String,
    pub fact: FileFact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InstanceBinding {
    SourceBound {
        records: Vec<(String, Option<String>)>,
    },
    Glob {
        pattern_matches: Vec<String>,
        root: Option<String>,
    },
}
