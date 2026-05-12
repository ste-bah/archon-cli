use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::extractor::{DeterministicExtractor, ExtractorConfig};
use crate::types::{ReasoningEventKind, ReasoningTurnInput};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabeledTurnFixture {
    pub fixture_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub assistant_text: String,
    pub expected_event_kinds: Vec<ReasoningEventKind>,
    pub forbidden_event_kinds: Vec<ReasoningEventKind>,
    pub code_fence_only: bool,
    pub quoted_user_only: bool,
}

impl Default for LabeledTurnFixture {
    fn default() -> Self {
        Self {
            fixture_id: String::new(),
            session_id: "fixture-session".to_string(),
            turn_number: 1,
            assistant_text: String::new(),
            expected_event_kinds: Vec::new(),
            forbidden_event_kinds: Vec::new(),
            code_fence_only: false,
            quoted_user_only: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureEvaluation {
    pub fixture_count: usize,
    pub true_positive: usize,
    pub false_positive: usize,
    pub false_negative: usize,
    pub claim_precision: f32,
    pub claim_recall: f32,
    pub claim_before_source_precision: f32,
    pub code_fence_false_positive_rate: f32,
    pub quoted_user_false_positive_rate: f32,
}

impl FixtureEvaluation {
    pub fn gates_pass(&self) -> bool {
        self.fixture_count >= 150
            && self.claim_precision >= 0.85
            && self.claim_recall >= 0.50
            && self.claim_before_source_precision >= 0.90
            && self.code_fence_false_positive_rate <= 0.05
            && self.quoted_user_false_positive_rate <= 0.05
    }
}

pub fn load_labeled_turns(dir: &Path) -> Result<Vec<LabeledTurnFixture>> {
    let mut fixtures = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let content = fs::read_to_string(entry.path())?;
        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            fixtures.push(serde_json::from_str(line)?);
        }
    }
    Ok(fixtures)
}

pub fn evaluate_labeled_turns(fixtures: &[LabeledTurnFixture]) -> FixtureEvaluation {
    let extractor = DeterministicExtractor::new(ExtractorConfig::default());
    let mut true_positive = 0;
    let mut false_positive = 0;
    let mut false_negative = 0;
    let mut before_source_tp = 0;
    let mut before_source_fp = 0;
    let mut code_fence_cases = 0;
    let mut code_fence_fp = 0;
    let mut quoted_cases = 0;
    let mut quoted_fp = 0;

    for fixture in fixtures {
        let input = ReasoningTurnInput {
            session_id: fixture.session_id.clone(),
            turn_number: fixture.turn_number,
            assistant_text: fixture.assistant_text.clone(),
            ..ReasoningTurnInput::default()
        };
        let observed: Vec<_> = extractor
            .extract_turn(&input)
            .into_iter()
            .map(|event| event.event_kind)
            .collect();

        for kind in &observed {
            if fixture.expected_event_kinds.contains(kind) {
                true_positive += 1;
            } else {
                false_positive += 1;
            }
            if *kind == ReasoningEventKind::ClaimBeforeSourceRead {
                if fixture.expected_event_kinds.contains(kind) {
                    before_source_tp += 1;
                } else {
                    before_source_fp += 1;
                }
            }
        }
        for expected in &fixture.expected_event_kinds {
            if !observed.contains(expected) {
                false_negative += 1;
            }
        }
        for forbidden in &fixture.forbidden_event_kinds {
            if observed.contains(forbidden) {
                false_positive += 1;
            }
        }
        if fixture.code_fence_only {
            code_fence_cases += 1;
            if !observed.is_empty() {
                code_fence_fp += 1;
            }
        }
        if fixture.quoted_user_only {
            quoted_cases += 1;
            if !observed.is_empty() {
                quoted_fp += 1;
            }
        }
    }

    FixtureEvaluation {
        fixture_count: fixtures.len(),
        true_positive,
        false_positive,
        false_negative,
        claim_precision: ratio(true_positive, true_positive + false_positive),
        claim_recall: ratio(true_positive, true_positive + false_negative),
        claim_before_source_precision: ratio(before_source_tp, before_source_tp + before_source_fp),
        code_fence_false_positive_rate: ratio(code_fence_fp, code_fence_cases),
        quoted_user_false_positive_rate: ratio(quoted_fp, quoted_cases),
    }
}

fn ratio(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f32 / denominator as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_eval_counts_expected_and_forbidden_events() {
        let fixtures = vec![LabeledTurnFixture {
            fixture_id: "f1".to_string(),
            assistant_text: "The module src/lib.rs exists.".to_string(),
            expected_event_kinds: vec![ReasoningEventKind::ClaimBeforeSourceRead],
            ..LabeledTurnFixture::default()
        }];
        let eval = evaluate_labeled_turns(&fixtures);
        assert_eq!(eval.true_positive, 1);
        assert_eq!(eval.false_positive, 0);
    }
}
