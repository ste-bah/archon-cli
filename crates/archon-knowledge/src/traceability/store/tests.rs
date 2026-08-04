use super::*;
use crate::schema::ensure_knowledge_schema;
use crate::traceability::anchors::{ANCHOR_RELATION_TYPE, anchor_relation};
use crate::traceability::requirements::{extract_requirements, requirement_entity};

fn db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").expect("in-memory cozo");
    ensure_knowledge_schema(&db).expect("kb schema");
    ensure_traceability_schema(&db).expect("traceability schema");
    db
}

fn anchor(req: &str, path: &str, start: usize) -> Anchor {
    Anchor {
        requirement_id: req.into(),
        task_id: "TASK-TDL-050".into(),
        file_path: path.into(),
        line_start: start,
        line_end: start + 9,
        file_hash: "5f4dcc3b5aa765d61d8327deb882cf99".into(),
        path_scope: path.into(),
        relevance_score: 0.62,
    }
}

#[test]
fn schema_creation_is_idempotent() {
    let db = db();
    ensure_traceability_schema(&db).expect("second create is a no-op");
    ensure_traceability_schema(&db).expect("third create is a no-op");
}

#[test]
fn an_anchor_round_trips_with_its_file_hash_and_level() {
    let db = db();
    let requirement = extract_requirements("- REQ-DL-034: Ingest Polygon natively.\n")
        .pop()
        .expect("requirement");
    let entity = requirement_entity(&requirement, "PRD.md");
    crate::store::insert_entity(&db, &entity).expect("entity");

    let anchor = anchor("REQ-DL-034", "src/ingest.rs", 40);
    let relation = anchor_relation(&anchor, &entity.entity_id);
    let record = AnchorRecord::from_anchor(&anchor, ProofLevel::Exercised, "2026-08-03T00:00:00Z");
    insert_anchor(&db, &relation, &record).expect("insert");

    let stored = list_anchors(&db).expect("list");
    assert_eq!(stored, vec![record.clone()]);
    assert_eq!(stored[0].citation(), "src/ingest.rs:40-49");
    assert_eq!(stored[0].file_hash, anchor.file_hash);
    assert_eq!(stored[0].proof_level, "Exercised");
    assert_eq!(stored[0].line_start, 40);
    assert_eq!(stored[0].line_end, 49);

    // The edge itself exists in kb_relations and names its citation.
    let edges = crate::store::list_relations(&db).expect("relations");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].relation_id, record.relation_id);
    assert_eq!(edges[0].relation_type, ANCHOR_RELATION_TYPE);
    assert_eq!(edges[0].source_entity_id, entity.entity_id);
    assert_eq!(edges[0].source_chunk_id, "src/ingest.rs:40-49");
}

#[test]
fn re_anchoring_the_same_span_updates_rather_than_duplicates() {
    let db = db();
    let anchor = anchor("REQ-DL-034", "src/ingest.rs", 40);
    let relation = anchor_relation(&anchor, "req-entity");
    insert_anchor(
        &db,
        &relation,
        &AnchorRecord::from_anchor(&anchor, ProofLevel::Candidate, "t0"),
    )
    .expect("first");
    insert_anchor(
        &db,
        &relation,
        &AnchorRecord::from_anchor(&anchor, ProofLevel::Exercised, "t1"),
    )
    .expect("second");

    let stored = list_anchors(&db).expect("list");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].proof_level, "Exercised");
}

#[test]
fn a_moved_anchor_is_a_new_edge_not_an_overwrite() {
    let db = db();
    for start in [40, 41] {
        let anchor = anchor("REQ-DL-034", "src/ingest.rs", start);
        insert_anchor(
            &db,
            &anchor_relation(&anchor, "req-entity"),
            &AnchorRecord::from_anchor(&anchor, ProofLevel::Candidate, "t0"),
        )
        .expect("insert");
    }
    assert_eq!(list_anchors(&db).expect("list").len(), 2);
}

#[test]
fn anchors_can_be_fetched_per_requirement() {
    let db = db();
    for (req, path) in [("REQ-DL-034", "src/a.rs"), ("REQ-DL-035", "src/b.rs")] {
        let anchor = anchor(req, path, 1);
        insert_anchor(
            &db,
            &anchor_relation(&anchor, "req-entity"),
            &AnchorRecord::from_anchor(&anchor, ProofLevel::Candidate, "t0"),
        )
        .expect("insert");
    }
    let found = anchors_for(&db, "REQ-DL-035").expect("filter");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].file_path, "src/b.rs");
    assert!(anchors_for(&db, "REQ-DL-999").expect("filter").is_empty());
}

#[test]
fn an_untraced_store_reports_no_anchors_rather_than_erroring() {
    // No `ensure_traceability_schema`: the relation does not exist.
    let bare = DbInstance::new("mem", "", "").expect("in-memory cozo");
    assert!(list_anchors(&bare).expect("empty, not an error").is_empty());
}
