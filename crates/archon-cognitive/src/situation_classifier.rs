use chrono::Utc;
use uuid::Uuid;

use crate::types::{
    ClassifierConfidence, CognitiveSurface, Situation, SituationKind, hash_user_text,
};

#[derive(Debug, Clone)]
pub struct ClassifyInput<'a> {
    pub user_text: &'a str,
    pub session_id: &'a str,
    pub turn_number: u64,
    pub surface: CognitiveSurface,
}

#[derive(Debug, Default, Clone)]
pub struct SituationClassifier;

impl SituationClassifier {
    pub fn classify(&self, input: ClassifyInput<'_>) -> Situation {
        let normalized = normalize(input.user_text);
        let tokens = token_count(&normalized);
        let (kind, score, reason) = classify_normalized(&normalized, tokens);
        Situation {
            id: Uuid::new_v4().to_string(),
            session_id: input.session_id.to_owned(),
            turn_number: input.turn_number,
            user_text_hash: hash_user_text(input.user_text),
            kind,
            confidence_score: score,
            confidence: ClassifierConfidence::from_score(score),
            reason,
            surface: input.surface,
            created_at: Utc::now(),
        }
    }
}

fn classify_normalized(text: &str, tokens: usize) -> (SituationKind, f32, String) {
    if is_greeting(text, tokens) {
        return hit(
            SituationKind::Greeting,
            0.95,
            "trivial greeting or acknowledgement",
        );
    }
    if has_any(text, HIGH_RISK) || destructive_pair(text) {
        return hit(
            SituationKind::HighRisk,
            0.9,
            "destructive or privileged operation",
        );
    }
    if has_any(text, GIT_MUTATION) {
        return hit(SituationKind::GitMutation, 0.88, "git mutation requested");
    }
    if is_ci_debug(text) {
        return hit(
            SituationKind::CiDebug,
            0.86,
            "CI or GitHub Actions debugging",
        );
    }
    if has_any(text, CODE_REVIEW) {
        return hit(
            SituationKind::Research,
            0.82,
            "codebase audit or inspection request",
        );
    }
    if has_any(text, PIPELINE_CONTROL) {
        return hit(
            SituationKind::PipelineControl,
            0.86,
            "pipeline control request",
        );
    }
    if has_any(text, WORLD_MODEL) {
        return hit(
            SituationKind::WorldModelTask,
            0.86,
            "world model or JEPA request",
        );
    }
    if has_any(text, CODE_CHANGE) {
        return hit(
            SituationKind::CodeChange,
            0.82,
            "code implementation request",
        );
    }
    if has_any(text, RESEARCH) {
        return hit(SituationKind::Research, 0.8, "research or evidence request");
    }
    if is_simple_question(text, tokens) {
        return hit(
            SituationKind::SimpleQuestion,
            0.72,
            "short informational question",
        );
    }
    hit(
        SituationKind::Ambiguous,
        0.35,
        "no deterministic rule matched",
    )
}

fn hit(kind: SituationKind, score: f32, reason: &str) -> (SituationKind, f32, String) {
    (kind, score, reason.to_owned())
}

fn normalize(text: &str) -> String {
    text.trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn token_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn is_greeting(text: &str, tokens: usize) -> bool {
    if tokens > 5 || looks_substantive(text) {
        return false;
    }
    GREETINGS.iter().any(|candidate| text == *candidate)
}

fn looks_substantive(text: &str) -> bool {
    text.ends_with('?') || has_any(text, REQUEST_MARKERS)
}

fn is_simple_question(text: &str, tokens: usize) -> bool {
    tokens <= 18
        && (text.ends_with('?')
            || text.starts_with("what ")
            || text.starts_with("why ")
            || text.starts_with("how ")
            || text.starts_with("can "))
}

fn is_ci_debug(text: &str) -> bool {
    has_word(text, "ci") || has_any(text, CI_DEBUG)
}

fn has_word(text: &str, needle: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == needle)
}

fn destructive_pair(text: &str) -> bool {
    (text.contains("delete") || text.contains("overwrite"))
        && has_any(text, &[" all", " everything", " database", " production"])
}

const GREETINGS: &[&str] = &[
    "hello",
    "hi",
    "hey",
    "yo",
    "hola",
    "howdy",
    "good morning",
    "good afternoon",
    "good evening",
    "what's up",
    "how are you",
    "nice to meet you",
    "ok",
    "okay",
    "thanks",
    "thank you",
    "got it",
    "understood",
    "bye",
    "goodbye",
    "see you",
    "later",
    "cya",
];

const REQUEST_MARKERS: &[&str] = &[
    " fix ",
    " build ",
    " run ",
    " check ",
    " create ",
    " implement ",
    " explain ",
    " why ",
];
const HIGH_RISK: &[&str] = &["force push", "--force", "drop table", "rm -rf", "chmod 777"];
const GIT_MUTATION: &[&str] = &["commit", "push", "merge", "rebase", "cherry-pick"];
const CI_DEBUG: &[&str] = &["github action", "actions run", "build failed"];
const CODE_REVIEW: &[&str] = &[
    "audit",
    "code review",
    "review the code",
    "whole repo",
    "codebase",
    "repo review",
    "inspect the repo",
];
const PIPELINE_CONTROL: &[&str] = &["pipeline", "resume", "rewind", "quality gate"];
const WORLD_MODEL: &[&str] = &["world model", "jepa", "train-jepa", "eval-jepa"];
const CODE_CHANGE: &[&str] = &["fix", "implement", "patch", "refactor", "edit"];
const RESEARCH: &[&str] = &["research", "source", "citation", "paper", "web search"];
