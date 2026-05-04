use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeaningLabel {
    Positive,
    Negative,
}

impl MeaningLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "positive" | "Positive" => Some(Self::Positive),
            "negative" | "Negative" => Some(Self::Negative),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeaningSample {
    pub sample_id: String,
    pub workspace_id: String,
    pub artifact_id: String,
    pub label: MeaningLabel,
    pub source_event_id: String,
    pub event_type: String,
    pub text: String,
    pub metadata_json: serde_json::Value,
    pub created_at: String,
}

pub fn classify_event(event_type: &str) -> Option<MeaningLabel> {
    match event_type {
        "UserAccepted" | "CompletionClaimVerified" => Some(MeaningLabel::Positive),
        "UserCorrected" | "FalseCompletionDetected" | "TestFailed" => Some(MeaningLabel::Negative),
        _ => None,
    }
}

pub fn sample_text(signal: &serde_json::Value, source_artifact_id: &str) -> String {
    for key in [
        "text",
        "output",
        "summary",
        "claim_text",
        "correction",
        "reason",
    ] {
        if let Some(value) = signal.get(key).and_then(serde_json::Value::as_str)
            && !value.trim().is_empty()
        {
            return value.trim().to_string();
        }
    }
    source_artifact_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_positive_and_negative_events() {
        assert_eq!(classify_event("UserAccepted"), Some(MeaningLabel::Positive));
        assert_eq!(
            classify_event("FalseCompletionDetected"),
            Some(MeaningLabel::Negative)
        );
        assert_eq!(classify_event("RetrievalUsed"), None);
    }

    #[test]
    fn extracts_text_from_signal() {
        let signal = serde_json::json!({"output": " useful answer "});
        assert_eq!(sample_text(&signal, "fallback"), "useful answer");
    }
}
