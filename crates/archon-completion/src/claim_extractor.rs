//! Extract completion-sensitive claims from agent/pipeline output text.
//!
//! Scans for phrases like "tests pass", "done", "implemented", "fixed",
//! "indexed", "cited" and converts them into structured [`CompletionClaim`]
//! records with inferred claim kinds.

use regex::Regex;

use super::models::{CompletionClaim, CompletionClaimKind, EvidenceKind};

/// Known completion-sensitive phrases and their corresponding claim kinds.
struct ClaimPattern {
    regex: Regex,
    kind: CompletionClaimKind,
    required_evidence: Vec<EvidenceKind>,
}

/// Build the pattern table once and reuse.
fn claim_patterns() -> Vec<ClaimPattern> {
    vec![
        ClaimPattern {
            // "tests pass" / "tests passing" / "all tests passed" etc.
            regex: Regex::new(r"(?i)\b(?:all\s+)?tests?\s+(?:pass|passed|passing|green|succeed(?:ed)?|all\s+green)\b").unwrap(),
            kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
        },
        ClaimPattern {
            // "build passes" / "build succeeded" / "compiles"
            regex: Regex::new(r"(?i)\b(?:build|compil(?:es?|ation))\s+(?:pass(?:es|ed)?|succeed(?:s|ed)?|green|clean)\b").unwrap(),
            kind: CompletionClaimKind::BuildPasses,
            required_evidence: vec![EvidenceKind::BuildResult],
        },
        ClaimPattern {
            // "implemented" / "implementation complete"
            regex: Regex::new(r"(?i)\b(?:implement(?:ed|ation)\s+(?:is\s+)?(?:done|complete|finished|ready)|fully\s+implemented)\b").unwrap(),
            kind: CompletionClaimKind::Implemented,
            required_evidence: vec![EvidenceKind::FileDiff, EvidenceKind::GeneratedArtifact],
        },
        ClaimPattern {
            // "fixed" / "resolved" / "patched"
            regex: Regex::new(r"(?i)\b(?:bug\s+)?(?:fix(?:ed)?|resolved|patch(?:ed)?)\s+(?:and\s+)?(?:verified|tested|deployed)?\b").unwrap(),
            kind: CompletionClaimKind::Fixed,
            required_evidence: vec![EvidenceKind::FileDiff, EvidenceKind::TestRun],
        },
        ClaimPattern {
            // "indexed" / "indexing complete"
            regex: Regex::new(r"(?i)\b(?:successfully\s+)?index(?:ed|ing)\s+(?:is\s+)?(?:done|complete|ready|successful)\b").unwrap(),
            kind: CompletionClaimKind::Indexed,
            required_evidence: vec![EvidenceKind::IngestionJob],
        },
        ClaimPattern {
            // "ingested" / "ingestion complete"
            regex: Regex::new(r"(?i)\b(?:successfully\s+)?ingest(?:ed|ion)\s+(?:is\s+)?(?:done|complete|ready|successful)\b").unwrap(),
            kind: CompletionClaimKind::Ingested,
            required_evidence: vec![EvidenceKind::IngestionJob],
        },
        ClaimPattern {
            // "cited" / "citation" — answer grounded in sources
            regex: Regex::new(r"(?i)\b(?:all\s+)?(?:claims?\s+)?(?:are\s+)?(?:cited|citation-grounded|grounded\s+in\s+sources?|source-backed|with\s+citations?)\b").unwrap(),
            kind: CompletionClaimKind::AnswerGrounded,
            required_evidence: vec![EvidenceKind::CitationTrace],
        },
        ClaimPattern {
            // "verified" / "confirmed"
            regex: Regex::new(r"(?i)\b(?:manually\s+)?(?:verif(?:ied|y)|confirm(?:ed)?)\s+(?:that\s+)?(?:it|the|all|everything)?\s*(?:works?|pass(?:es|ed)?|is\s+correct)?").unwrap(),
            kind: CompletionClaimKind::Verified,
            required_evidence: vec![EvidenceKind::ReviewFinding],
        },
        ClaimPattern {
            // "documented"
            regex: Regex::new(r"(?i)\b(?:fully\s+)?document(?:ed|ation)\s+(?:is\s+)?(?:done|complete|ready|written)\b").unwrap(),
            kind: CompletionClaimKind::Documented,
            required_evidence: vec![EvidenceKind::GeneratedArtifact],
        },
        ClaimPattern {
            // "done" / "complete" / "finished" — catch-all, MUST be last
            regex: Regex::new(r"(?i)\b(?:task\s+is\s+)?(?:done|complete|finished|ready)\s*[.!]?\s*$").unwrap(),
            kind: CompletionClaimKind::Done,
            required_evidence: vec![EvidenceKind::GateResult],
        },
    ]
}

/// Negation patterns that precede a claim and invalidate it.
fn negation_prefix() -> Regex {
    Regex::new(r"(?i)\b(?:not|no|don't|doesn't|isn't|aren't|haven't|hasn't|won't|can't|cannot|never|fail(?:ed|s)?\s+to|unable\s+to)\s+").unwrap()
}

/// Extract completion claims from arbitrary text (agent output, report, etc.).
///
/// Returns claims with `claim_id` and `run_id` placeholders — caller fills in
/// the real identifiers before persistence.
pub fn extract_claims(text: &str, run_id: &str) -> Vec<CompletionClaim> {
    let patterns = claim_patterns();
    let neg_prefix = negation_prefix();
    let now = chrono::Utc::now().to_rfc3339();
    let mut claims = Vec::new();

    // Split into sentences for context-window negation check.
    let sentences: Vec<&str> = text.split(&['.', '!', '?', '\n'][..]).collect();

    for sentence in &sentences {
        let s = sentence.trim();
        if s.is_empty() || s.len() < 4 {
            continue;
        }

        for pattern in &patterns {
            if let Some(m) = pattern.regex.find(s) {
                // Check for negation in the preceding context (within the same sentence).
                let prefix = &s[..m.start()];
                if neg_prefix.is_match(prefix) {
                    continue; // negated — don't emit a claim
                }

                let claim_id = format!(
                    "cl-{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
                );

                claims.push(CompletionClaim {
                    claim_id,
                    run_id: run_id.to_string(),
                    agent_key: None,
                    model: None,
                    task_type: "pipeline-output".into(),
                    claim_text: m.as_str().to_string(),
                    claim_kind: pattern.kind.clone(),
                    required_evidence: pattern.required_evidence.clone(),
                    linked_evidence_ids: vec![],
                    verified: false,
                    contradiction_ids: vec![],
                    created_at: now.clone(),
                });
                break; // one claim kind per sentence
            }
        }
    }

    claims
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recognizes_tests_pass() {
        let claims = extract_claims("All tests pass.", "run-1");
        assert!(claims.iter().any(|c| c.claim_kind == CompletionClaimKind::TestsPass));
    }

    #[test]
    fn test_recognizes_all_kinds() {
        let text = "\
            All tests pass. \
            The build passes cleanly. \
            Implementation is done. \
            Bug fixed and verified. \
            Indexing is complete. \
            Ingestion is done. \
            All claims are citation-grounded. \
            Task is done. \
            Verified that it works. \
            Documentation is complete.";

        let claims = extract_claims(text, "run-1");
        let kinds: Vec<CompletionClaimKind> = claims.iter().map(|c| c.claim_kind.clone()).collect();
        assert!(kinds.contains(&CompletionClaimKind::TestsPass), "must detect TestsPass");
        assert!(kinds.contains(&CompletionClaimKind::BuildPasses), "must detect BuildPasses");
        assert!(kinds.contains(&CompletionClaimKind::Implemented), "must detect Implemented");
        assert!(kinds.contains(&CompletionClaimKind::Fixed), "must detect Fixed");
        assert!(kinds.contains(&CompletionClaimKind::Indexed), "must detect Indexed");
        assert!(kinds.contains(&CompletionClaimKind::Ingested), "must detect Ingested");
        assert!(kinds.contains(&CompletionClaimKind::AnswerGrounded), "must detect AnswerGrounded");
        assert!(kinds.contains(&CompletionClaimKind::Done), "must detect Done");
        assert!(kinds.contains(&CompletionClaimKind::Verified), "must detect Verified");
        assert!(kinds.contains(&CompletionClaimKind::Documented), "must detect Documented");
    }

    #[test]
    fn test_ignores_negated_phrases() {
        let claims = extract_claims("Tests do not pass. The tests don't pass.", "run-1");
        assert!(!claims.iter().any(|c| c.claim_kind == CompletionClaimKind::TestsPass),
            "negated tests-pass must not produce claims");
    }

    #[test]
    fn test_ignores_failed_to_patterns() {
        let claims = extract_claims("Failed to implement the feature.", "run-1");
        assert!(!claims.iter().any(|c| c.claim_kind == CompletionClaimKind::Implemented),
            "failed-to must not produce Implemented claim");
    }

    #[test]
    fn test_empty_input() {
        let claims = extract_claims("", "run-1");
        assert!(claims.is_empty());
    }
}
