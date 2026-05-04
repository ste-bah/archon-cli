use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceEdgeType {
    DerivedFrom,
    Contains,
    ExtractedFrom,
    Describes,
    Cites,
    GeneratedBy,
    Used,
}

impl ProvenanceEdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DerivedFrom => "DerivedFrom",
            Self::Contains => "Contains",
            Self::ExtractedFrom => "ExtractedFrom",
            Self::Describes => "Describes",
            Self::Cites => "Cites",
            Self::GeneratedBy => "GeneratedBy",
            Self::Used => "Used",
        }
    }

    pub fn parse(value: &str) -> Self {
        let normalized = value
            .chars()
            .filter(|ch| !matches!(ch, '-' | '_' | ' '))
            .flat_map(char::to_lowercase)
            .collect::<String>();
        match normalized.as_str() {
            "contains" => Self::Contains,
            "extractedfrom" => Self::ExtractedFrom,
            "describes" => Self::Describes,
            "cites" => Self::Cites,
            "generatedby" => Self::GeneratedBy,
            "used" => Self::Used,
            _ => Self::DerivedFrom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_edge_type_case_insensitively() {
        assert_eq!(
            ProvenanceEdgeType::parse("contains"),
            ProvenanceEdgeType::Contains
        );
        assert_eq!(
            ProvenanceEdgeType::parse("extracted_from"),
            ProvenanceEdgeType::ExtractedFrom
        );
        assert_eq!(
            ProvenanceEdgeType::parse("Generated-By"),
            ProvenanceEdgeType::GeneratedBy
        );
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub record_id: String,
    pub artifact_id: String,
    pub artifact_type: String,
    pub operation: String,
    pub input_hashes: Vec<String>,
    pub output_hash: String,
    pub parent_record_ids: Vec<String>,
    pub tool_name: Option<String>,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub parameters_json: serde_json::Value,
    pub timestamp: String,
    pub chain_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceEdge {
    pub edge_id: String,
    pub from_artifact_id: String,
    pub to_artifact_id: String,
    pub edge_type: ProvenanceEdgeType,
    pub created_at: String,
}

impl ProvenanceEdge {
    pub fn new(
        from_artifact_id: &str,
        to_artifact_id: &str,
        edge_type: ProvenanceEdgeType,
    ) -> Self {
        Self {
            edge_id: format!("edge-{}", uuid::Uuid::new_v4()),
            from_artifact_id: from_artifact_id.to_string(),
            to_artifact_id: to_artifact_id.to_string(),
            edge_type,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}
