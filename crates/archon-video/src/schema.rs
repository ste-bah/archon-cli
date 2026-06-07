use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

pub const CREATE_VIDEO_SOURCES: &str = r#":create video_sources {
    video_id: String =>
    document_id: String,
    source_kind: String,
    source_url: String,
    local_path: String default "",
    title: String default "",
    channel_or_author: String default "",
    duration_ms: Int default 0,
    published_at: String default "",
    license: String default "",
    source_hash: String,
    ingest_status: String,
    policy_snapshot_json: String,
    created_at: String,
    updated_at: String,
}"#;

pub const CREATE_VIDEO_TRACKS: &str = r#":create video_tracks {
    track_id: String =>
    video_id: String,
    track_kind: String,
    provider: String,
    model: String default "",
    status: String,
    warning_count: Int default 0,
    error_count: Int default 0,
    created_at: String,
    updated_at: String,
}"#;

pub const CREATE_VIDEO_TRANSCRIPT_SEGMENTS: &str = r#":create video_transcript_segments {
    segment_id: String =>
    video_id: String,
    track_id: String,
    start_ms: Int,
    end_ms: Int,
    speaker: String default "",
    text: String,
    confidence: Float default -1.0,
    source_method: String,
    chunk_id: String default "",
    created_at: String,
}"#;

pub const CREATE_VIDEO_FRAME_DESCRIPTIONS: &str = r#":create video_frame_descriptions {
    frame_id: String =>
    video_id: String,
    track_id: String,
    timestamp_ms: Int,
    timestamp_end_ms: Int,
    frame_hash: String,
    perceptual_hash: String default "",
    image_artifact_id: String default "",
    ocr_text: String default "",
    vlm_description: String default "",
    provider: String default "",
    model: String default "",
    cost_usd: Float default 0.0,
    chunk_id: String default "",
    dedupe_group_id: String default "",
    status: String,
    warning: String default "",
    created_at: String,
}"#;

pub const CREATE_VIDEO_CHUNK_TIMEREF: &str = r#":create video_chunk_timeref {
    chunk_id: String =>
    video_id: String,
    track_id: String,
    timestamp_start_ms: Int,
    timestamp_end_ms: Int,
    created_at: String,
}"#;

const VIDEO_SCHEMA: &[&str] = &[
    CREATE_VIDEO_SOURCES,
    CREATE_VIDEO_TRACKS,
    CREATE_VIDEO_TRANSCRIPT_SEGMENTS,
    CREATE_VIDEO_FRAME_DESCRIPTIONS,
    CREATE_VIDEO_CHUNK_TIMEREF,
];

pub fn create_video_schema(db: &DbInstance) -> Result<()> {
    for script in VIDEO_SCHEMA {
        run_create(db, script)?;
    }
    Ok(())
}

fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match archon_docs::run_cozo_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Mutable,
        "create video schema",
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if archon_docs::errors::COZO_RELATION_ALREADY_EXISTS
                .iter()
                .any(|phrase| msg.contains(phrase))
            {
                Ok(())
            } else {
                Err(anyhow::anyhow!("video schema creation failed: {msg}"))
            }
        }
    }
}
