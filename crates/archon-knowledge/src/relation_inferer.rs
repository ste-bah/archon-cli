use crate::schema::{EntityRecord, RelationRecord};
use crate::store::DocumentChunk;
use crate::{now_iso, stable_id};

pub fn infer_relations(chunk: &DocumentChunk, entities: &[EntityRecord]) -> Vec<RelationRecord> {
    let mut relations = Vec::new();
    for pair in entities.windows(2) {
        let left = &pair[0];
        let right = &pair[1];
        if left.entity_id == right.entity_id {
            continue;
        }
        let relation_type = infer_relation_type(&chunk.content, &left.name, &right.name);
        relations.push(RelationRecord {
            relation_id: stable_id(
                "relation",
                &[
                    &chunk.chunk_id,
                    &left.entity_id,
                    &right.entity_id,
                    relation_type,
                ],
            ),
            source_entity_id: left.entity_id.clone(),
            target_entity_id: right.entity_id.clone(),
            relation_type: relation_type.to_string(),
            source_chunk_id: chunk.chunk_id.clone(),
            confidence: if relation_type == "co_mentions" {
                0.55
            } else {
                0.78
            },
            created_at: now_iso(),
        });
    }
    relations
}

fn infer_relation_type(content: &str, left: &str, right: &str) -> &'static str {
    let lower = content.to_lowercase();
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    if lower.contains(&format!("{left} uses {right}")) {
        "uses"
    } else if lower.contains(&format!("{left} requires {right}")) {
        "requires"
    } else if lower.contains(&format!("{left} supports {right}")) {
        "supports"
    } else {
        "co_mentions"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: &str, name: &str) -> EntityRecord {
        EntityRecord {
            entity_id: id.into(),
            name: name.into(),
            entity_type: "concept".into(),
            source_chunk_id: "chunk-1".into(),
            mentions: 1,
            confidence: 0.8,
            created_at: "now".into(),
        }
    }

    #[test]
    fn infers_typed_relation_when_phrase_matches() {
        let chunk = DocumentChunk {
            chunk_id: "chunk-1".into(),
            document_id: "doc-1".into(),
            content: "Archon uses CozoDB for evidence.".into(),
            content_hash: "hash".into(),
        };
        let relations = infer_relations(&chunk, &[entity("e1", "Archon"), entity("e2", "CozoDB")]);
        assert_eq!(relations[0].relation_type, "uses");
    }

    #[test]
    fn falls_back_to_co_mentions() {
        let chunk = DocumentChunk {
            chunk_id: "chunk-1".into(),
            document_id: "doc-1".into(),
            content: "Archon and CozoDB appear together.".into(),
            content_hash: "hash".into(),
        };
        let relations = infer_relations(&chunk, &[entity("e1", "Archon"), entity("e2", "CozoDB")]);
        assert_eq!(relations[0].relation_type, "co_mentions");
    }

    #[test]
    fn no_relation_for_single_entity() {
        let chunk = DocumentChunk {
            chunk_id: "chunk-1".into(),
            document_id: "doc-1".into(),
            content: "Archon.".into(),
            content_hash: "hash".into(),
        };
        assert!(infer_relations(&chunk, &[entity("e1", "Archon")]).is_empty());
    }
}
