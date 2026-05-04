use std::collections::BTreeMap;

use crate::schema::EntityRecord;
use crate::store::DocumentChunk;
use crate::{now_iso, stable_id};

pub fn extract_entities(chunk: &DocumentChunk) -> Vec<EntityRecord> {
    let mut mentions: BTreeMap<String, i64> = BTreeMap::new();
    for candidate in entity_candidates(&chunk.content) {
        *mentions.entry(candidate).or_default() += 1;
    }
    mentions
        .into_iter()
        .map(|(name, count)| EntityRecord {
            entity_id: stable_id("entity", &[&chunk.chunk_id, &name]),
            entity_type: classify_entity(&name).to_string(),
            source_chunk_id: chunk.chunk_id.clone(),
            confidence: if count > 1 { 0.86 } else { 0.72 },
            mentions: count,
            created_at: now_iso(),
            name,
        })
        .collect()
}

fn entity_candidates(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for token in content.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| c.is_ascii_punctuation());
        if is_entity_token(cleaned) && !is_sentence_starter_noise(cleaned) {
            current.push(cleaned.to_string());
            continue;
        }
        flush_candidate(&mut current, &mut out);
    }
    flush_candidate(&mut current, &mut out);
    out
}

fn flush_candidate(current: &mut Vec<String>, out: &mut Vec<String>) {
    if current.is_empty() {
        return;
    }
    let candidate = current.join(" ");
    if candidate.len() > 1 {
        out.push(candidate);
    }
    current.clear();
}

fn is_entity_token(token: &str) -> bool {
    let has_upper = token.chars().any(char::is_uppercase);
    let all_caps = token.len() > 1 && token.chars().all(|c| c.is_ascii_uppercase() || c == '-');
    has_upper || all_caps
}

fn is_sentence_starter_noise(token: &str) -> bool {
    matches!(
        token,
        "The" | "A" | "An" | "This" | "That" | "It" | "These" | "Those"
    )
}

fn classify_entity(name: &str) -> &'static str {
    if name.ends_with("Inc") || name.ends_with("Ltd") || name.ends_with("Labs") {
        "organization"
    } else if name.chars().all(|c| c.is_ascii_uppercase() || c == '-') {
        "acronym"
    } else {
        "concept"
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
    fn extracts_capitalized_entities() {
        let entities = extract_entities(&chunk("Archon evaluates Policy Pack documents."));
        assert!(entities.iter().any(|e| e.name == "Archon"));
        assert!(entities.iter().any(|e| e.name == "Policy Pack"));
    }

    #[test]
    fn counts_repeated_mentions() {
        let entities = extract_entities(&chunk("Archon improves Archon memory."));
        let archon = entities.iter().find(|e| e.name == "Archon").unwrap();
        assert_eq!(archon.mentions, 2);
    }

    #[test]
    fn classifies_acronyms() {
        let entities = extract_entities(&chunk("OCR enables document intelligence."));
        assert_eq!(entities[0].entity_type, "acronym");
    }
}
