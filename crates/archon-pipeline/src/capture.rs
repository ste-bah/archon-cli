//! Autonomous memory capture -- pattern-based detection of corrections,
//! decisions, error patterns, preferences, and project state.
//! Implements REQ-PIPE-015.

use regex::RegexSet;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CaptureType {
    Correction,
    Decision,
    ErrorPattern,
    Preference,
    ProjectState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedMemory {
    pub content: String,
    pub memory_type: CaptureType,
    pub source_turn: usize,
    pub confidence: f32,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// AutoCapture
// ---------------------------------------------------------------------------

pub struct AutoCapture {
    enabled: bool,
    patterns: HashMap<CaptureType, RegexSet>,
}

impl AutoCapture {
    pub fn new(enabled: bool) -> Self {
        let mut patterns = HashMap::new();

        patterns.insert(
            CaptureType::Correction,
            RegexSet::new([
                r"(?i)no[,.]?\s+(do it|use|try)",
                r"(?i)don'?t\s+do\s+",
                r"(?i)that'?s\s+wrong",
                r"(?i)instead[,.]?\s+",
                r"(?i)stop\s+doing",
                r"(?i)I\s+said\s+not\s+to",
                r"(?i)not\s+like\s+that",
                r"(?i)wrong\s+approach",
            ])
            .expect("correction patterns"),
        );

        patterns.insert(
            CaptureType::Decision,
            RegexSet::new([
                r"(?i)let'?s\s+go\s+with",
                r"(?i)the\s+architecture\s+will",
                r"(?i)we\s+decided\s+to",
                r"(?i)the\s+approach\s+is",
                r"(?i)use\s+\w+\s+for\s+",
                r"(?i)we'?re\s+going\s+with",
            ])
            .expect("decision patterns"),
        );

        patterns.insert(
            CaptureType::ErrorPattern,
            RegexSet::new([
                r"(?i)this\s+error\s+means",
                r"(?i)when\s+you\s+see\s+\w+\s+do",
                r"(?i)the\s+fix\s+for\s+this\s+is",
                r"(?i)root\s+cause\s+was",
                r"(?i)the\s+error\s+is\s+caused\s+by",
            ])
            .expect("error patterns"),
        );

        patterns.insert(
            CaptureType::Preference,
            RegexSet::new([
                r"(?i)I\s+prefer\s+",
                r"(?i)always\s+use\s+",
                r"(?i)never\s+do\s+",
                r"(?i)my\s+workflow\s+is",
                r"(?i)I\s+like\s+\w+\s+over\s+",
            ])
            .expect("preference patterns"),
        );

        patterns.insert(
            CaptureType::ProjectState,
            RegexSet::new([
                r"(?i)the\s+deadline\s+is",
                r"(?i)scope\s+changed\s+to",
                r"(?i)blocked\s+by\s+",
                r"(?i)we'?re\s+freezing",
                r"(?i)priority\s+shifted\s+to",
            ])
            .expect("project state patterns"),
        );

        Self { enabled, patterns }
    }

    /// Detect capturable information from a user message.
    pub fn detect(&self, user_message: &str, turn_index: usize) -> Vec<CapturedMemory> {
        if !self.enabled || user_message.trim().is_empty() {
            return vec![];
        }

        let mut captures = Vec::new();
        let timestamp = chrono::Utc::now().to_rfc3339();

        for (capture_type, regex_set) in &self.patterns {
            let matches: Vec<usize> = regex_set.matches(user_message).into_iter().collect();
            if !matches.is_empty() {
                let content = Self::extract_relevant_content(user_message);
                let confidence =
                    Self::calculate_confidence(matches.len(), user_message.len());

                captures.push(CapturedMemory {
                    content,
                    memory_type: capture_type.clone(),
                    source_turn: turn_index,
                    confidence,
                    timestamp: timestamp.clone(),
                });
            }
        }

        captures
    }

    /// Check if a memory is a near-duplicate of existing memories (Jaccard > 0.8).
    pub fn is_duplicate(new: &CapturedMemory, existing: &[CapturedMemory]) -> bool {
        for mem in existing {
            if mem.memory_type != new.memory_type {
                continue;
            }
            let new_words: HashSet<&str> = new.content.split_whitespace().collect();
            let existing_words: HashSet<&str> = mem.content.split_whitespace().collect();

            if new_words.is_empty() || existing_words.is_empty() {
                continue;
            }

            let intersection = new_words.intersection(&existing_words).count();
            let union = new_words.union(&existing_words).count();

            if union > 0 {
                let jaccard = intersection as f64 / union as f64;
                if jaccard > 0.8 {
                    return true;
                }
            }
        }
        false
    }

    fn extract_relevant_content(message: &str) -> String {
        if message.len() <= 500 {
            message.to_string()
        } else {
            let cut = message
                .char_indices()
                .nth(497)
                .map(|(i, _)| i)
                .unwrap_or(message.len());
            format!("{}...", &message[..cut])
        }
    }

    fn calculate_confidence(match_count: usize, message_len: usize) -> f32 {
        let base: f64 = match match_count {
            1 => 0.6,
            2 => 0.75,
            _ => 0.85,
        };
        let length_factor: f64 = if message_len < 100 {
            1.0
        } else if message_len < 300 {
            0.95
        } else {
            0.9
        };
        (base * length_factor).min(1.0) as f32
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_capture() -> AutoCapture {
        AutoCapture::new(true)
    }

    // -- detect corrections --------------------------------------------------

    #[test]
    fn detect_correction() {
        let ac = make_capture();
        let caps = ac.detect("No, don't do that. Use the other approach instead.", 1);
        let types: HashSet<_> = caps.iter().map(|c| &c.memory_type).collect();
        assert!(
            types.contains(&CaptureType::Correction),
            "should detect correction, got: {:?}",
            types
        );
    }

    #[test]
    fn detect_decision() {
        let ac = make_capture();
        let caps = ac.detect("Let's go with PostgreSQL for the database.", 2);
        let types: HashSet<_> = caps.iter().map(|c| &c.memory_type).collect();
        assert!(types.contains(&CaptureType::Decision));
    }

    #[test]
    fn detect_preference() {
        let ac = make_capture();
        let caps = ac.detect("I prefer Rust over Go for systems work.", 3);
        let types: HashSet<_> = caps.iter().map(|c| &c.memory_type).collect();
        assert!(types.contains(&CaptureType::Preference));
    }

    #[test]
    fn detect_error_pattern() {
        let ac = make_capture();
        let caps = ac.detect("The root cause was a missing null check in the parser.", 4);
        let types: HashSet<_> = caps.iter().map(|c| &c.memory_type).collect();
        assert!(types.contains(&CaptureType::ErrorPattern));
    }

    #[test]
    fn detect_project_state() {
        let ac = make_capture();
        let caps = ac.detect("We're freezing the API for the v2 release.", 5);
        let types: HashSet<_> = caps.iter().map(|c| &c.memory_type).collect();
        assert!(types.contains(&CaptureType::ProjectState));
    }

    // -- trivial messages return empty ---------------------------------------

    #[test]
    fn detect_trivial_hello() {
        let ac = make_capture();
        assert!(ac.detect("hello", 0).is_empty());
    }

    #[test]
    fn detect_trivial_thanks() {
        let ac = make_capture();
        assert!(ac.detect("thanks", 0).is_empty());
    }

    #[test]
    fn detect_trivial_ok() {
        let ac = make_capture();
        assert!(ac.detect("ok", 0).is_empty());
    }

    #[test]
    fn detect_empty_string() {
        let ac = make_capture();
        assert!(ac.detect("", 0).is_empty());
    }

    #[test]
    fn detect_whitespace_only() {
        let ac = make_capture();
        assert!(ac.detect("   \n  ", 0).is_empty());
    }

    // -- deduplication -------------------------------------------------------

    #[test]
    fn is_duplicate_catches_near_identical() {
        let mem1 = CapturedMemory {
            content: "Use PostgreSQL for the database layer".into(),
            memory_type: CaptureType::Decision,
            source_turn: 1,
            confidence: 0.7,
            timestamp: "t1".into(),
        };
        let mem2 = CapturedMemory {
            content: "Use PostgreSQL for the database layer please".into(),
            memory_type: CaptureType::Decision,
            source_turn: 2,
            confidence: 0.7,
            timestamp: "t2".into(),
        };
        assert!(AutoCapture::is_duplicate(&mem2, &[mem1]));
    }

    #[test]
    fn is_duplicate_different_content() {
        let mem1 = CapturedMemory {
            content: "Use PostgreSQL for the database".into(),
            memory_type: CaptureType::Decision,
            source_turn: 1,
            confidence: 0.7,
            timestamp: "t1".into(),
        };
        let mem2 = CapturedMemory {
            content: "The frontend should use React with TypeScript".into(),
            memory_type: CaptureType::Decision,
            source_turn: 2,
            confidence: 0.7,
            timestamp: "t2".into(),
        };
        assert!(!AutoCapture::is_duplicate(&mem2, &[mem1]));
    }

    #[test]
    fn is_duplicate_different_types_not_dup() {
        let mem1 = CapturedMemory {
            content: "Use PostgreSQL for the database layer".into(),
            memory_type: CaptureType::Decision,
            source_turn: 1,
            confidence: 0.7,
            timestamp: "t1".into(),
        };
        let mem2 = CapturedMemory {
            content: "Use PostgreSQL for the database layer".into(),
            memory_type: CaptureType::Preference,
            source_turn: 2,
            confidence: 0.7,
            timestamp: "t2".into(),
        };
        assert!(!AutoCapture::is_duplicate(&mem2, &[mem1]));
    }

    // -- disabled capture returns empty --------------------------------------

    #[test]
    fn disabled_returns_empty() {
        let ac = AutoCapture::new(false);
        let caps = ac.detect("No, don't do that. Wrong approach!", 1);
        assert!(caps.is_empty());
    }

    // -- confidence is higher for short direct messages ----------------------

    #[test]
    fn confidence_higher_for_short_messages() {
        let ac = make_capture();
        let short = ac.detect("That's wrong", 1);
        let long_msg = format!(
            "That's wrong, and here is a very long explanation about why {} end.",
            "word ".repeat(60)
        );
        let long = ac.detect(&long_msg, 2);

        assert!(!short.is_empty(), "short should match");
        assert!(!long.is_empty(), "long should match");
        assert!(
            short[0].confidence >= long[0].confidence,
            "short confidence ({}) should be >= long confidence ({})",
            short[0].confidence,
            long[0].confidence,
        );
    }
}
