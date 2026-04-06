use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A single memory node in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub title: String,
    pub memory_type: MemoryType,
    pub importance: f64,
    pub tags: Vec<String>,
    pub source_type: String,
    pub project_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub access_count: u64,
    pub last_accessed: Option<DateTime<Utc>>,
}

/// Categorisation of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryType {
    Fact,
    Decision,
    Correction,
    Pattern,
    Preference,
    Rule,
    PersonalitySnapshot,
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Fact => "fact",
            Self::Decision => "decision",
            Self::Correction => "correction",
            Self::Pattern => "pattern",
            Self::Preference => "preference",
            Self::Rule => "rule",
            Self::PersonalitySnapshot => "personality_snapshot",
        };
        f.write_str(s)
    }
}

impl MemoryType {
    /// Parse from a stored string. Returns `None` for unknown values.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "fact" => Some(Self::Fact),
            "decision" => Some(Self::Decision),
            "correction" => Some(Self::Correction),
            "pattern" => Some(Self::Pattern),
            "preference" => Some(Self::Preference),
            "rule" => Some(Self::Rule),
            "personality_snapshot" => Some(Self::PersonalitySnapshot),
            _ => None,
        }
    }
}

impl Memory {
    /// Returns the number of days since this memory was last accessed.
    /// Returns None if last_accessed is not set.
    pub fn days_since_access(&self) -> Option<i64> {
        self.last_accessed
            .map(|la| (chrono::Utc::now() - la).num_days())
    }
}

/// A directed edge between two memory nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub from_id: String,
    pub to_id: String,
    pub rel_type: RelType,
    pub context: Option<String>,
    pub strength: f64,
    pub created_at: DateTime<Utc>,
}

/// Relationship type between two memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelType {
    RelatedTo,
    CausedBy,
    Contradicts,
    Supersedes,
    DerivedFrom,
}

impl fmt::Display for RelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::RelatedTo => "related_to",
            Self::CausedBy => "caused_by",
            Self::Contradicts => "contradicts",
            Self::Supersedes => "supersedes",
            Self::DerivedFrom => "derived_from",
        };
        f.write_str(s)
    }
}

impl RelType {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "related_to" => Some(Self::RelatedTo),
            "caused_by" => Some(Self::CausedBy),
            "contradicts" => Some(Self::Contradicts),
            "supersedes" => Some(Self::Supersedes),
            "derived_from" => Some(Self::DerivedFrom),
            _ => None,
        }
    }
}

/// Filter criteria for structured memory search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchFilter {
    pub memory_type: Option<MemoryType>,
    pub tags: Vec<String>,
    /// When true, ALL tags must match; when false, ANY tag matches.
    pub require_all_tags: bool,
    pub text: Option<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

/// Errors produced by the memory subsystem.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("memory not found: {0}")]
    NotFound(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("invalid memory type: {0}")]
    InvalidType(String),

    #[error("invalid relationship type: {0}")]
    InvalidRelType(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("embedding error: {0}")]
    Embedding(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_type_round_trip() {
        for mt in [
            MemoryType::Fact,
            MemoryType::Decision,
            MemoryType::Correction,
            MemoryType::Pattern,
            MemoryType::Preference,
            MemoryType::Rule,
            MemoryType::PersonalitySnapshot,
        ] {
            let s = mt.to_string();
            let parsed = MemoryType::from_str_opt(&s).expect("should parse");
            assert_eq!(parsed, mt);
        }
    }

    #[test]
    fn rel_type_round_trip() {
        for rt in [
            RelType::RelatedTo,
            RelType::CausedBy,
            RelType::Contradicts,
            RelType::Supersedes,
            RelType::DerivedFrom,
        ] {
            let s = rt.to_string();
            let parsed = RelType::from_str_opt(&s).expect("should parse");
            assert_eq!(parsed, rt);
        }
    }

    #[test]
    fn invalid_types_return_none() {
        assert!(MemoryType::from_str_opt("bogus").is_none());
        assert!(RelType::from_str_opt("bogus").is_none());
    }

    #[test]
    fn search_filter_default_is_empty() {
        let f = SearchFilter::default();
        assert!(f.memory_type.is_none());
        assert!(f.tags.is_empty());
        assert!(!f.require_all_tags);
        assert!(f.text.is_none());
        assert!(f.date_from.is_none());
        assert!(f.date_to.is_none());
    }

    #[test]
    fn memory_error_display() {
        let e = MemoryError::NotFound("abc".into());
        assert!(e.to_string().contains("abc"));
        let e = MemoryError::InvalidType("xyz".into());
        assert!(e.to_string().contains("xyz"));
    }

    // ── days_since_access tests (TASK-CLI-417) ──────────────────

    fn make_test_memory(last_accessed: Option<DateTime<Utc>>) -> Memory {
        Memory {
            id: "test".into(),
            content: "test".into(),
            title: "test".into(),
            memory_type: MemoryType::Fact,
            importance: 0.5,
            tags: vec![],
            source_type: "test".into(),
            project_path: "".into(),
            created_at: Utc::now(),
            updated_at: None,
            access_count: 0,
            last_accessed,
        }
    }

    #[test]
    fn days_since_access_returns_none_when_not_set() {
        let mem = make_test_memory(None);
        assert_eq!(mem.days_since_access(), None);
    }

    #[test]
    fn days_since_access_returns_zero_for_now() {
        let mem = make_test_memory(Some(Utc::now()));
        assert_eq!(mem.days_since_access(), Some(0));
    }

    #[test]
    fn days_since_access_returns_positive_for_past() {
        let five_days_ago = Utc::now() - chrono::Duration::days(5);
        let mem = make_test_memory(Some(five_days_ago));
        assert_eq!(mem.days_since_access(), Some(5));
    }
}
