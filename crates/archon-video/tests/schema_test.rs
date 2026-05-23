use archon_video::schema::create_video_schema;
use cozo::{DbInstance, ScriptMutability};

#[test]
fn create_video_schema_creates_all_relations_and_is_idempotent() {
    let db = DbInstance::new("mem", "", "").expect("in-memory CozoDB");

    create_video_schema(&db).expect("first schema creation");
    create_video_schema(&db).expect("second schema creation should be idempotent");

    for (relation, key) in [
        ("video_sources", "video_id"),
        ("video_tracks", "track_id"),
        ("video_transcript_segments", "segment_id"),
        ("video_frame_descriptions", "frame_id"),
        ("video_chunk_timeref", "chunk_id"),
    ] {
        let script = format!("?[id] := *{relation}{{{key}: id}}");
        let result = db.run_script(&script, Default::default(), ScriptMutability::Immutable);
        assert!(
            result.is_ok(),
            "relation {relation} should exist: {result:?}"
        );
        assert!(result.unwrap().rows.is_empty());
    }
}
