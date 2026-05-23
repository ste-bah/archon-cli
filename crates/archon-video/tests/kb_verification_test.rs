use archon_docs::models::ChunkArtifact;
use archon_video::schema::create_video_schema;
use archon_video::store::{self, ChunkTimeRef};
use cozo::DbInstance;

#[test]
fn kb_process_auto_consumes_video_chunks() {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    archon_docs::store::insert_chunk(
        &db,
        &ChunkArtifact {
            chunk_id: "chunk-video-kb".into(),
            document_id: "doc-video-kb".into(),
            artifact_id: "artifact-video-transcript".into(),
            chunk_index: 0,
            page_start: 0,
            page_end: 0,
            content: "Disposition scorecards require reason codes.".into(),
            content_hash: "hash-video-kb".into(),
            embedding_status: "pending".into(),
        },
    )
    .unwrap();
    store::insert_chunk_timeref(
        &db,
        &ChunkTimeRef {
            chunk_id: "chunk-video-kb".into(),
            video_id: "video-kb".into(),
            track_id: "track-kb".into(),
            timestamp_start_ms: 12_000,
            timestamp_end_ms: 18_000,
            created_at: "2026-05-22T00:00:00Z".into(),
        },
    )
    .unwrap();

    let chunks = archon_knowledge::store::list_doc_chunks(&db).unwrap();
    assert!(
        chunks
            .iter()
            .any(|chunk| chunk.chunk_id == "chunk-video-kb")
    );

    let engine = archon_knowledge::KnowledgeEngine::new(db).unwrap();
    let report = engine
        .process_documents(archon_knowledge::ProcessOptions {
            claims: true,
            entities: false,
            relations: false,
            contradictions: false,
        })
        .unwrap();
    assert_eq!(report.chunks_seen, 1);
    assert!(report.claims_created > 0);
}
