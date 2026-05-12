use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;

use crate::types::ReasoningQualityEvent;

pub fn ledger_path(root: &Path) -> PathBuf {
    let date = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    root.join("events").join(format!("{date}.jsonl"))
}

pub fn append_events(root: &Path, events: &[ReasoningQualityEvent]) -> Result<PathBuf> {
    let path = ledger_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    for event in events {
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use crate::extractor::{DeterministicExtractor, ExtractorConfig};
    use crate::types::ReasoningTurnInput;

    use super::*;

    #[test]
    fn appends_one_json_line_per_event() {
        let temp = tempfile::tempdir().unwrap();
        let input = ReasoningTurnInput {
            session_id: "s1".to_string(),
            turn_number: 1,
            assistant_text: "The module src/lib.rs exists.".to_string(),
            ..ReasoningTurnInput::default()
        };
        let events = DeterministicExtractor::new(ExtractorConfig::default()).extract_turn(&input);
        let path = append_events(temp.path(), &events).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), events.len());
        assert!(content.contains("\"event_kind\":\"claim_before_source_read\""));
    }
}
