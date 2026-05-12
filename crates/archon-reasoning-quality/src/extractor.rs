use chrono::Utc;
use regex::Regex;

use crate::canonical::{
    CANONICALIZER_VERSION, canonicalize_claim_text, claim_id_for, event_id_for, hash_hex,
};
use crate::evidence::match_claim_evidence;
use crate::redaction::{RedactionConfig, redact_entity_key, redact_text};
use crate::severity::{base_severity, effective_severity};
use crate::types::{
    ConfidenceSignal, ReasoningClaim, ReasoningEventKind, ReasoningQualityEvent, ReasoningSubject,
    ReasoningTurnInput, VerificationState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractorConfig {
    pub max_claims_per_turn: usize,
    pub max_excerpt_chars: usize,
    pub shadow: bool,
}

impl Default for ExtractorConfig {
    fn default() -> Self {
        Self {
            max_claims_per_turn: 12,
            max_excerpt_chars: 600,
            shadow: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicExtractor {
    config: ExtractorConfig,
}

impl DeterministicExtractor {
    pub fn new(config: ExtractorConfig) -> Self {
        Self { config }
    }

    pub fn extract_turn(&self, input: &ReasoningTurnInput) -> Vec<ReasoningQualityEvent> {
        let redaction = RedactionConfig {
            allow_raw_text: input.store_raw_text,
            workspace_root: input.workspace_root.clone(),
            max_excerpt_chars: self.config.max_excerpt_chars,
            ..RedactionConfig::default()
        };
        let raw_hash = Some(hash_hex(&input.assistant_text));
        let mut events = Vec::new();

        for sentence in visible_sentences(&input.assistant_text) {
            if events.len() >= self.config.max_claims_per_turn {
                break;
            }
            let Some(subject) = classify_subject(&sentence) else {
                continue;
            };
            let confidence = classify_confidence(&sentence);
            let canonical_text = canonicalize_claim_text(&sentence);
            if canonical_text.is_empty() {
                continue;
            }
            let entity_key = redact_entity_key(&extract_entity_key(subject, &sentence), &redaction);
            let claim_id = claim_id_for(
                &input.session_id,
                input.turn_number,
                &canonical_text,
                subject,
                &entity_key,
            );
            let evidence = match_claim_evidence(subject, &entity_key, &input.evidence_refs);
            let event_kind = classify_event_kind(subject, confidence, &sentence, evidence.state);
            let severity_base = base_severity(event_kind);
            let severity_effective =
                effective_severity(event_kind, confidence, None).unwrap_or(0.0);
            let claim = ReasoningClaim {
                claim_id: claim_id.clone(),
                canonicalizer_version: CANONICALIZER_VERSION.to_string(),
                canonical_text: canonical_text.clone(),
                subject,
                entity_key: entity_key.clone(),
                confidence_signal: confidence,
                turn_number: input.turn_number,
            };
            events.push(ReasoningQualityEvent {
                event_id: event_id_for(
                    &input.session_id,
                    input.turn_number,
                    &claim.claim_id,
                    &format!("{event_kind:?}"),
                ),
                session_id: input.session_id.clone(),
                turn_number: input.turn_number,
                claim_id,
                event_kind,
                subject,
                entity_key,
                canonicalizer_version: CANONICALIZER_VERSION.to_string(),
                canonical_text,
                confidence_signal: confidence,
                verification_state: verification_for_event(event_kind, evidence.state),
                severity_base,
                severity_effective,
                evidence_refs: evidence.refs,
                redacted_excerpt: Some(redact_text(&sentence, &redaction)),
                raw_text_hash: raw_hash.clone(),
                shadow: self.config.shadow,
                created_at: Utc::now(),
                ..ReasoningQualityEvent::default()
            });
        }

        events
    }
}

fn classify_event_kind(
    subject: ReasoningSubject,
    confidence: ConfidenceSignal,
    sentence: &str,
    evidence_state: VerificationState,
) -> ReasoningEventKind {
    if evidence_state == VerificationState::VerifiedBeforeClaim {
        return ReasoningEventKind::SourceVerifiedClaim;
    }
    if matches!(
        confidence,
        ConfidenceSignal::Uncertain | ConfidenceSignal::ExplicitlySpeculative
    ) {
        return ReasoningEventKind::UncertaintyDisclosed;
    }
    if subject == ReasoningSubject::TestStatus && looks_like_test_status(sentence) {
        return ReasoningEventKind::TestStatusClaimWithoutCommand;
    }
    if subject == ReasoningSubject::CompletionStatus {
        return ReasoningEventKind::CompletionClaimWithoutEvidence;
    }
    if matches!(
        subject,
        ReasoningSubject::Codebase
            | ReasoningSubject::Configuration
            | ReasoningSubject::Documentation
            | ReasoningSubject::ProviderStatus
            | ReasoningSubject::RuntimeStatus
    ) {
        return ReasoningEventKind::ClaimBeforeSourceRead;
    }
    ReasoningEventKind::UnsupportedClaim
}

fn verification_for_event(
    event_kind: ReasoningEventKind,
    evidence_state: VerificationState,
) -> VerificationState {
    match event_kind {
        ReasoningEventKind::SourceVerifiedClaim => VerificationState::VerifiedBeforeClaim,
        ReasoningEventKind::UncertaintyDisclosed => VerificationState::NotRequired,
        _ => evidence_state,
    }
}

fn visible_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || trimmed.starts_with('>') || trimmed.starts_with("User:") {
            continue;
        }
        if trimmed.len() > 8 {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn classify_subject(sentence: &str) -> Option<ReasoningSubject> {
    let lower = sentence.to_lowercase();
    if looks_like_test_status(sentence) || lower.contains("cargo test") {
        Some(ReasoningSubject::TestStatus)
    } else if lower.contains("done")
        || lower.contains("complete")
        || lower.contains("implemented")
        || lower.contains("finished")
    {
        Some(ReasoningSubject::CompletionStatus)
    } else if lower.contains("provider")
        || lower.contains("codex")
        || lower.contains("anthropic")
        || lower.contains("openai")
        || lower.contains("oauth")
    {
        Some(ReasoningSubject::ProviderStatus)
    } else if lower.contains("config")
        || lower.contains("policy")
        || lower.contains("setting")
        || lower.contains("toml")
    {
        Some(ReasoningSubject::Configuration)
    } else if lower.contains("readme") || lower.contains("docs/") || lower.contains("documentation")
    {
        Some(ReasoningSubject::Documentation)
    } else if lower.contains("plan") || lower.contains("task list") {
        Some(ReasoningSubject::Plan)
    } else if codebase_regex().is_match(sentence) {
        Some(ReasoningSubject::Codebase)
    } else if lower.contains("architecture") || lower.contains("design") {
        Some(ReasoningSubject::ArchitectureAdvice)
    } else {
        None
    }
}

fn classify_confidence(sentence: &str) -> ConfidenceSignal {
    let lower = sentence.to_lowercase();
    if lower.contains("speculat") || lower.contains("guess") || lower.contains("hypothetical") {
        ConfidenceSignal::ExplicitlySpeculative
    } else if lower.contains("i think") || lower.contains("maybe") || lower.contains("possibly") {
        ConfidenceSignal::Uncertain
    } else if lower.contains("likely") || lower.contains("appears") || lower.contains("seems") {
        ConfidenceSignal::Qualified
    } else if confident_regex().is_match(sentence) {
        ConfidenceSignal::Confident
    } else {
        ConfidenceSignal::Unknown
    }
}

fn extract_entity_key(subject: ReasoningSubject, sentence: &str) -> String {
    match subject {
        ReasoningSubject::Codebase | ReasoningSubject::Documentation => path_regex()
            .find(sentence)
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| subject_tag(subject)),
        ReasoningSubject::Configuration => config_key_regex()
            .find(sentence)
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "config".to_string()),
        ReasoningSubject::TestStatus => test_name_regex()
            .find(sentence)
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "test-status".to_string()),
        ReasoningSubject::ProviderStatus => provider_regex()
            .find(sentence)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_else(|| "provider".to_string()),
        _ => subject_tag(subject),
    }
}

fn subject_tag(subject: ReasoningSubject) -> String {
    format!("{subject:?}").to_lowercase()
}

fn looks_like_test_status(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    (lower.contains("test") || lower.contains("build")) && status_regex().is_match(&lower)
}

fn codebase_regex() -> Regex {
    Regex::new(r"\b(src|crates|tests|docs)/[A-Za-z0-9_./-]+|\b(crate|module|function|struct|enum|impl|codebase)\b").unwrap()
}

fn path_regex() -> Regex {
    Regex::new(r"\b(?:src|crates|tests|docs)/[A-Za-z0-9_./-]+").unwrap()
}

fn config_key_regex() -> Regex {
    Regex::new(r"\b[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+\b|[A-Za-z0-9_.-]+\.toml").unwrap()
}

fn test_name_regex() -> Regex {
    Regex::new(r"\b[A-Za-z0-9_:.-]*test[A-Za-z0-9_:.-]*\b").unwrap()
}

fn provider_regex() -> Regex {
    Regex::new(r"(?i)\b(codex|anthropic|openai|gemini|ollama|provider)\b").unwrap()
}

fn confident_regex() -> Regex {
    Regex::new(r"(?i)\b(is|are|does|has|will|must|always|never|ships|passes|fails|exists)\b")
        .unwrap()
}

fn status_regex() -> Regex {
    Regex::new(r"\b(pass|passes|passed|fail|fails|failed|green|red|working|broken)\b").unwrap()
}

#[cfg(test)]
mod tests {
    use crate::types::{EvidenceKind, EvidenceRef};

    use super::*;

    #[test]
    fn emits_codebase_claim_before_source_read() {
        let input = ReasoningTurnInput {
            session_id: "s1".to_string(),
            turn_number: 1,
            assistant_text: "The module src/command/world_model.rs handles provider routing."
                .to_string(),
            ..ReasoningTurnInput::default()
        };
        let events = DeterministicExtractor::new(ExtractorConfig::default()).extract_turn(&input);
        assert_eq!(
            events[0].event_kind,
            ReasoningEventKind::ClaimBeforeSourceRead
        );
    }

    #[test]
    fn verified_source_read_softens_to_source_verified_claim() {
        let evidence = EvidenceRef {
            evidence_id: "ev1".to_string(),
            kind: EvidenceKind::FileRead,
            entity_key: Some("src/command/world_model.rs".to_string()),
            ..EvidenceRef::default()
        };
        let input = ReasoningTurnInput {
            session_id: "s1".to_string(),
            turn_number: 2,
            assistant_text: "The module src/command/world_model.rs exists.".to_string(),
            evidence_refs: vec![evidence],
            ..ReasoningTurnInput::default()
        };
        let events = DeterministicExtractor::new(ExtractorConfig::default()).extract_turn(&input);
        assert_eq!(
            events[0].event_kind,
            ReasoningEventKind::SourceVerifiedClaim
        );
    }

    #[test]
    fn skips_code_fences_and_user_quotes() {
        let input = ReasoningTurnInput {
            session_id: "s1".to_string(),
            turn_number: 3,
            assistant_text: "> src/fake.rs exists\n```text\nsrc/fake.rs exists\n```".to_string(),
            ..ReasoningTurnInput::default()
        };
        let events = DeterministicExtractor::new(ExtractorConfig::default()).extract_turn(&input);
        assert!(events.is_empty());
    }
}
