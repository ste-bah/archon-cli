use crate::types::ReasoningSubject;

pub const CANONICALIZER_VERSION: &str = "canonicalize_v1";

pub fn canonicalize_claim_text(text: &str) -> String {
    let without_fences = strip_code_fences(text);
    without_fences
        .lines()
        .filter(|line| !is_quoted_user_line(line))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c.is_ascii_punctuation() && c != '/')
        .to_lowercase()
}

pub fn claim_id_for(
    session_id: &str,
    turn_number: u64,
    canonical_text: &str,
    subject: ReasoningSubject,
    entity_key: &str,
) -> String {
    let input = format!(
        "{CANONICALIZER_VERSION}\n{canonical_text}\n{session_id}\n{turn_number}\n{subject:?}\n{entity_key}"
    );
    format!("rqclm_{}", short_hash(&input))
}

pub fn event_id_for(
    session_id: &str,
    turn_number: u64,
    claim_id: &str,
    event_kind: &str,
) -> String {
    let input = format!("{session_id}\n{turn_number}\n{claim_id}\n{event_kind}");
    format!("rqevt_{}", short_hash(&input))
}

pub fn hash_hex(input: &str) -> String {
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

fn short_hash(input: &str) -> String {
    hash_hex(input).chars().take(16).collect()
}

fn strip_code_fences(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn is_quoted_user_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>') || trimmed.starts_with("User:") || trimmed.starts_with("user:")
}

#[cfg(test)]
mod tests {
    use crate::types::ReasoningSubject;

    use super::*;

    #[test]
    fn canonicalize_skips_fences_and_user_quotes() {
        let text =
            "> user says src/main.rs exists\nThe module exists.\n```rust\nassert!(false)\n```";
        assert_eq!(canonicalize_claim_text(text), "the module exists");
    }

    #[test]
    fn claim_id_is_deterministic() {
        let left = claim_id_for(
            "s1",
            2,
            "the module exists",
            ReasoningSubject::Codebase,
            "src/lib.rs",
        );
        let right = claim_id_for(
            "s1",
            2,
            "the module exists",
            ReasoningSubject::Codebase,
            "src/lib.rs",
        );
        assert_eq!(left, right);
        assert!(left.starts_with("rqclm_"));
    }
}
