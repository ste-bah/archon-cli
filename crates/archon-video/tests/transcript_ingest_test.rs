use archon_policy::EffectivePolicy;
use archon_video::ingest::{IngestOpts, ingest_video};
use archon_video::schema::create_video_schema;
use cozo::{DbInstance, ScriptMutability};
use serde_json::Value;

fn test_db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    db
}

fn policy() -> EffectivePolicy {
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;
    policy.video.allow_youtube = true;
    policy.video.allow_direct_urls = true;
    policy
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[tokio::test]
async fn ingests_local_video_transcript_into_docs_and_video_relations() {
    let db = test_db();
    let result = ingest_video(
        IngestOpts {
            source: "mini_lecture.mp4".into(),
            transcript_path: Some(fixture("mini_lecture.vtt")),
            metadata_only: false,
            frames_mode: None,
            asr_provider: None,
            vlm: false,
            yes: true,
        },
        &policy(),
        &db,
    )
    .await
    .unwrap();

    assert!(result.was_new);
    assert_eq!(result.chunk_count, 5);
    assert!(
        archon_video::store::get_video_source(&db, &result.video_id)
            .unwrap()
            .is_some()
    );
    assert!(
        archon_docs::store::get_doc_source(&db, &result.document_id)
            .unwrap()
            .is_some()
    );
    let artifacts = archon_docs::store::list_artifacts_for_doc(&db, &result.document_id).unwrap();
    assert!(
        artifacts
            .iter()
            .any(|a| a.artifact_type == "video_transcript")
    );
    let chunks = archon_docs::store::list_chunks_for_doc(&db, &result.document_id).unwrap();
    assert_eq!(chunks.len(), 5);
    assert_eq!(count(&db, "video_transcript_segments", "segment_id"), 5);
    assert_eq!(count(&db, "video_chunk_timeref", "chunk_id"), 5);
    assert!(
        !archon_docs::store::list_provenance_from(&db, &chunks[0].chunk_id)
            .unwrap()
            .is_empty()
    );

    let duplicate = ingest_video(
        IngestOpts {
            source: "mini_lecture.mp4".into(),
            transcript_path: Some(fixture("mini_lecture.vtt")),
            metadata_only: false,
            frames_mode: None,
            asr_provider: None,
            vlm: false,
            yes: true,
        },
        &policy(),
        &db,
    )
    .await
    .unwrap();
    assert!(!duplicate.was_new);
    assert_eq!(duplicate.video_id, result.video_id);
}

#[tokio::test]
async fn transcript_only_youtube_does_not_require_acquisition() {
    let db = test_db();
    let result = ingest_video(
        IngestOpts {
            source: "https://www.youtube.com/watch?v=abc123".into(),
            transcript_path: Some(fixture("mini_lecture.vtt")),
            metadata_only: true,
            frames_mode: None,
            asr_provider: None,
            vlm: false,
            yes: true,
        },
        &policy(),
        &db,
    )
    .await
    .unwrap();

    let video = archon_video::store::get_video_source(&db, &result.video_id)
        .unwrap()
        .unwrap();
    let snapshot: Value = serde_json::from_str(&video.policy_snapshot_json).unwrap();
    assert_eq!(video.source_kind, "YouTube");
    assert_eq!(snapshot["acquisition_method"], "None");
    assert_eq!(
        archon_docs::store::list_chunks_for_doc(&db, &result.document_id)
            .unwrap()
            .len(),
        5
    );
}

fn count(db: &DbInstance, relation: &str, key: &str) -> i64 {
    let script = format!("?[count(id)] := *{relation}{{{key}: id}}");
    let result = db
        .run_script(&script, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    result.rows[0][0].get_int().unwrap_or(0)
}
