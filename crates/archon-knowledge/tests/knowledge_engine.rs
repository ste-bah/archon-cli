use archon_docs::models::{ChunkArtifact, DocumentStatus, SourceDocument};
use archon_knowledge::schema::{
    ClaimPolarity, ClaimRecord, ContradictionRecord, EntityRecord, RelationRecord,
};
use archon_knowledge::source_quality::{self, QualityOutcome};
use archon_knowledge::{KnowledgeEngine, ProcessOptions};
use cozo::DbInstance;

fn db_with_doc_schema() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    db
}

fn engine(db: DbInstance) -> KnowledgeEngine {
    KnowledgeEngine::new(db).unwrap()
}

fn insert_chunk(db: &DbInstance, id: &str, doc: &str, content: &str) {
    let chunk = ChunkArtifact {
        chunk_id: id.into(),
        document_id: doc.into(),
        artifact_id: format!("artifact-{id}"),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: content.into(),
        content_hash: format!("hash-{id}"),
        embedding_status: "pending".into(),
    };
    archon_docs::store::insert_chunk(db, &chunk).unwrap();
}

fn insert_source(db: &DbInstance, doc: &str) {
    archon_docs::store::insert_doc_source(
        db,
        &SourceDocument {
            document_id: doc.into(),
            source_path: format!("/tmp/{doc}.md"),
            media_type: "text/markdown".into(),
            content_hash: format!("hash-{doc}"),
            discovered_at: "2026-05-27T00:00:00Z".into(),
            status: DocumentStatus::Ingested,
        },
    )
    .unwrap();
}

#[test]
fn process_fixture_persists_claims_entities_relations_quality_and_contradictions() {
    let db = db_with_doc_schema();
    insert_chunk(
        &db,
        "c1",
        "doc-1",
        "Archon uses CozoDB. The plugin is safe.",
    );
    insert_chunk(
        &db,
        "c2",
        "doc-2",
        "The plugin is not safe. Policy Pack supports OCR.",
    );
    let engine = engine(db);

    let report = engine.process_documents(ProcessOptions::default()).unwrap();

    assert_eq!(report.chunks_seen, 2);
    assert!(report.claims_created >= 3);
    assert!(report.entities_created >= 3);
    assert!(report.relations_created >= 1);
    assert_eq!(report.source_quality_records, 2);
    assert_eq!(report.contradictions_created, 1);
    assert_eq!(engine.contradictions().unwrap().len(), 1);
}

#[test]
fn claim_crud_roundtrip() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);
    archon_knowledge::store::insert_claim(engine.db(), &claim("claim-1", ClaimPolarity::Positive))
        .unwrap();
    let rows = engine.claims().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].normalized_subject, "plugin");
}

#[test]
fn entity_crud_roundtrip() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);
    archon_knowledge::store::insert_entity(engine.db(), &entity("entity-1", "Archon")).unwrap();
    let rows = engine.entities().unwrap();
    assert_eq!(rows[0].name, "Archon");
}

#[test]
fn relation_crud_roundtrip() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);
    archon_knowledge::store::insert_relation(engine.db(), &relation("relation-1")).unwrap();
    let rows = engine.relations().unwrap();
    assert_eq!(rows[0].relation_type, "uses");
}

#[test]
fn contradiction_crud_roundtrip() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);
    archon_knowledge::store::insert_contradiction(engine.db(), &contradiction("contra-1")).unwrap();
    let rows = engine.contradictions().unwrap();
    assert!(rows[0].explanation.contains("conflict"));
}

#[test]
fn source_quality_outcome_update_persists() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);
    let record =
        source_quality::update_from_outcome(engine.db(), "doc-1", QualityOutcome::UserAccepted)
            .unwrap();
    let persisted = archon_knowledge::store::get_source_quality(engine.db(), "doc-1")
        .unwrap()
        .unwrap();
    assert_eq!(persisted.observations, 1);
    assert_eq!(persisted.score, record.score);
}

#[test]
fn process_kb_only_reads_attached_documents() {
    let db = db_with_doc_schema();
    insert_source(&db, "doc-1");
    insert_source(&db, "doc-2");
    archon_docs::store::assign_document_to_kb(&db, "trading", "doc-1").unwrap();
    insert_chunk(&db, "c1", "doc-1", "The scorecard is safe.");
    insert_chunk(&db, "c2", "doc-2", "The unrelated policy is safe.");
    let engine = engine(db);

    let report = engine
        .process_kb(
            "trading",
            ProcessOptions {
                claims: true,
                entities: false,
                relations: false,
                contradictions: false,
            },
        )
        .unwrap();

    assert_eq!(report.chunks_seen, 1);
    assert_eq!(engine.claims().unwrap().len(), 1);
}

#[test]
fn process_claims_only_skips_entities_and_relations() {
    let db = db_with_doc_schema();
    insert_chunk(
        &db,
        "c1",
        "doc-1",
        "The plugin is safe. Archon uses CozoDB.",
    );
    let engine = engine(db);
    let report = engine
        .process_documents(ProcessOptions::from_flags(true, false, false, false))
        .unwrap();
    assert!(report.claims_created > 0);
    assert_eq!(report.entities_created, 0);
    assert_eq!(report.relations_created, 0);
}

#[test]
fn process_empty_doc_chunks_is_safe() {
    let db = db_with_doc_schema();
    let engine = engine(db);
    let report = engine.process_documents(ProcessOptions::default()).unwrap();
    assert_eq!(report.chunks_seen, 0);
    assert_eq!(engine.stats().unwrap().claims, 0);
}

#[test]
fn no_process_flags_default_to_all_extractors() {
    let db = db_with_doc_schema();
    insert_chunk(
        &db,
        "c1",
        "doc-1",
        "Archon uses CozoDB. The plugin is safe.",
    );
    let engine = engine(db);
    let report = engine
        .process_documents(ProcessOptions::from_flags(false, false, false, false))
        .unwrap();
    assert!(report.claims_created > 0);
    assert!(report.entities_created > 0);
}

#[test]
fn stats_reflect_persisted_rows() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);
    archon_knowledge::store::insert_claim(engine.db(), &claim("claim-1", ClaimPolarity::Positive))
        .unwrap();
    archon_knowledge::store::insert_entity(engine.db(), &entity("entity-1", "Archon")).unwrap();
    archon_knowledge::store::insert_relation(engine.db(), &relation("relation-1")).unwrap();
    let stats = engine.stats().unwrap();
    assert_eq!(stats.claims, 1);
    assert_eq!(stats.entities, 1);
    assert_eq!(stats.relations, 1);
}

fn claim(id: &str, polarity: ClaimPolarity) -> ClaimRecord {
    ClaimRecord {
        claim_id: id.into(),
        chunk_id: "chunk".into(),
        document_id: "doc".into(),
        text: "The plugin is safe".into(),
        normalized_subject: "plugin".into(),
        normalized_predicate: "safe".into(),
        polarity,
        confidence: 0.9,
        created_at: "now".into(),
    }
}

fn entity(id: &str, name: &str) -> EntityRecord {
    EntityRecord {
        entity_id: id.into(),
        name: name.into(),
        entity_type: "concept".into(),
        source_chunk_id: "chunk".into(),
        mentions: 1,
        confidence: 0.8,
        created_at: "now".into(),
    }
}

fn relation(id: &str) -> RelationRecord {
    RelationRecord {
        relation_id: id.into(),
        source_entity_id: "entity-1".into(),
        target_entity_id: "entity-2".into(),
        relation_type: "uses".into(),
        source_chunk_id: "chunk".into(),
        confidence: 0.8,
        created_at: "now".into(),
    }
}

fn contradiction(id: &str) -> ContradictionRecord {
    ContradictionRecord {
        contradiction_id: id.into(),
        left_claim_id: "claim-1".into(),
        right_claim_id: "claim-2".into(),
        contradiction_type: "opposite".into(),
        explanation: "claims conflict".into(),
        confidence: 0.9,
        created_at: "now".into(),
    }
}
