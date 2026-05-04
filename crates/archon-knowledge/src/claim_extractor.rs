use crate::schema::{ClaimPolarity, ClaimRecord};
use crate::store::DocumentChunk;
use crate::{now_iso, stable_id};

pub fn extract_claims(chunk: &DocumentChunk) -> Vec<ClaimRecord> {
    split_sentences(&chunk.content)
        .into_iter()
        .filter_map(|sentence| parse_claim(&sentence, chunk))
        .collect()
}

fn parse_claim(sentence: &str, chunk: &DocumentChunk) -> Option<ClaimRecord> {
    let cleaned = sentence
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation());
    if cleaned.split_whitespace().count() < 3 {
        return None;
    }

    let lower = normalize_space(&cleaned.to_lowercase());
    let parsed = parse_copula(&lower).or_else(|| parse_action_claim(&lower))?;
    if parsed.subject.is_empty() || parsed.predicate.is_empty() {
        return None;
    }

    let claim_id = stable_id("claim", &[&chunk.chunk_id, cleaned]);
    Some(ClaimRecord {
        claim_id,
        chunk_id: chunk.chunk_id.clone(),
        document_id: chunk.document_id.clone(),
        text: cleaned.to_string(),
        normalized_subject: parsed.subject,
        normalized_predicate: parsed.predicate,
        polarity: parsed.polarity,
        confidence: parsed.confidence,
        created_at: now_iso(),
    })
}

#[derive(Debug)]
struct ParsedClaim {
    subject: String,
    predicate: String,
    polarity: ClaimPolarity,
    confidence: f64,
}

fn parse_copula(lower: &str) -> Option<ParsedClaim> {
    for marker in [
        " is not ",
        " are not ",
        " was not ",
        " were not ",
        " cannot be ",
    ] {
        if let Some((subject, predicate)) = lower.split_once(marker) {
            return Some(parsed(subject, predicate, ClaimPolarity::Negative, 0.92));
        }
    }
    for marker in [" is ", " are ", " was ", " were ", " can be "] {
        if let Some((subject, predicate)) = lower.split_once(marker) {
            return Some(parsed(subject, predicate, ClaimPolarity::Positive, 0.88));
        }
    }
    None
}

fn parse_action_claim(lower: &str) -> Option<ParsedClaim> {
    for marker in [" does not ", " do not ", " cannot ", " never "] {
        if let Some((subject, rest)) = lower.split_once(marker) {
            return Some(parsed(subject, rest, ClaimPolarity::Negative, 0.84));
        }
    }
    for verb in [
        "supports", "support", "requires", "require", "uses", "use", "enables", "enable",
        "prevents", "prevent", "allows", "allow",
    ] {
        let marker = format!(" {verb} ");
        if let Some((subject, object)) = lower.split_once(&marker) {
            let predicate = format!("{} {}", canonical_verb(verb), normalize_phrase(object));
            return Some(ParsedClaim {
                subject: normalize_phrase(subject),
                predicate,
                polarity: ClaimPolarity::Positive,
                confidence: 0.82,
            });
        }
    }
    None
}

fn parsed(subject: &str, predicate: &str, polarity: ClaimPolarity, confidence: f64) -> ParsedClaim {
    ParsedClaim {
        subject: normalize_phrase(subject),
        predicate: normalize_phrase(predicate),
        polarity,
        confidence,
    }
}

fn split_sentences(content: &str) -> Vec<String> {
    content
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_phrase(input: &str) -> String {
    let words: Vec<&str> = input
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| c.is_ascii_punctuation()))
        .filter(|w| !w.is_empty())
        .filter(|w| !matches!(*w, "the" | "a" | "an"))
        .collect();
    normalize_space(&words.join(" "))
}

fn normalize_space(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn canonical_verb(verb: &str) -> &'static str {
    match verb {
        "supports" | "support" => "support",
        "requires" | "require" => "require",
        "uses" | "use" => "use",
        "enables" | "enable" => "enable",
        "prevents" | "prevent" => "prevent",
        "allows" | "allow" => "allow",
        _ => "relate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(content: &str) -> DocumentChunk {
        DocumentChunk {
            chunk_id: "chunk-1".into(),
            document_id: "doc-1".into(),
            content: content.into(),
            content_hash: "hash".into(),
        }
    }

    #[test]
    fn extracts_positive_copula_claim() {
        let claims = extract_claims(&chunk("The plugin is safe."));
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].normalized_subject, "plugin");
        assert_eq!(claims[0].normalized_predicate, "safe");
        assert_eq!(claims[0].polarity, ClaimPolarity::Positive);
    }

    #[test]
    fn extracts_negative_copula_claim() {
        let claims = extract_claims(&chunk("The plugin is not safe."));
        assert_eq!(claims[0].polarity, ClaimPolarity::Negative);
        assert_eq!(claims[0].normalized_predicate, "safe");
    }

    #[test]
    fn extracts_action_claim_with_canonical_verb() {
        let claims = extract_claims(&chunk("Archon supports local OCR."));
        assert_eq!(claims[0].normalized_subject, "archon");
        assert_eq!(claims[0].normalized_predicate, "support local ocr");
    }
}
