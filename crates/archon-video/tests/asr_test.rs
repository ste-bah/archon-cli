use std::io::Write;

use archon_policy::EffectivePolicy;
use archon_video::asr::{
    AsrOptions, AsrProvider, MockAsrAdapter, NullAsrAdapter, extract_audio_track,
};
use archon_video::errors::VideoError;
use archon_video::ingest::{IngestOpts, ingest_video_with_asr_provider};
use archon_video::schema::create_video_schema;
use archon_video::transcript::TranscriptSegment;
use cozo::{DbInstance, ScriptMutability};

fn segment() -> TranscriptSegment {
    TranscriptSegment {
        start_ms: 1000,
        end_ms: 2500,
        text: "ASR transcript segment".into(),
        confidence: Some(0.9),
        speaker: None,
    }
}

#[tokio::test]
async fn mock_asr_returns_valid_segments() {
    let adapter = MockAsrAdapter {
        segments: vec![segment()],
    };
    let segments = adapter
        .transcribe(
            b"audio",
            &AsrOptions {
                model: "base".into(),
                device: "cpu".into(),
                language: None,
                vad_stable_timestamps: false,
                diarization: false,
            },
        )
        .await
        .unwrap();

    assert!(segments[0].start_ms > 0);
    assert!(segments[0].end_ms > segments[0].start_ms);
}

#[tokio::test]
async fn null_asr_returns_structured_unavailable_error() {
    let err = NullAsrAdapter::default()
        .transcribe(b"", &AsrOptions::from(&EffectivePolicy::default().video))
        .await
        .unwrap_err();

    assert!(matches!(err, VideoError::AsrProviderUnavailable { .. }));
}

#[tokio::test]
async fn extract_audio_track_uses_mock_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("ffmpeg-mock.sh");
    write_script(
        &script,
        r#"#!/bin/sh
	for arg do out="$arg"; done
	printf 'RIFF' > "$out"
"#,
    );

    let wav = extract_audio_track(
        std::path::Path::new("video.mp4"),
        &script.display().to_string(),
    )
    .await
    .unwrap();

    assert!(std::fs::metadata(wav.path()).unwrap().len() > 0);
}

#[tokio::test]
async fn ingest_with_mock_asr_persists_local_asr_segments() {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;
    let adapter = MockAsrAdapter {
        segments: vec![segment()],
    };

    let result = ingest_video_with_asr_provider(
        IngestOpts {
            source: "asr_fixture.mp4".into(),
            transcript_path: None,
            metadata_only: false,
            frames_mode: None,
            asr_provider: Some("mock".into()),
            vlm: false,
            yes: true,
        },
        &policy,
        &db,
        Some(&adapter),
    )
    .await
    .unwrap();

    assert_eq!(result.chunk_count, 1);
    assert_eq!(source_method_count(&db, "local_asr"), 1);
}

fn source_method_count(db: &DbInstance, method: &str) -> i64 {
    let mut params = std::collections::BTreeMap::new();
    params.insert("method".into(), cozo::DataValue::from(method));
    let result = db
        .run_script(
            "?[count(segment_id)] := *video_transcript_segments{segment_id, source_method}, source_method = $method",
            params,
            ScriptMutability::Immutable,
        )
        .unwrap();
    result.rows[0][0].get_int().unwrap_or(0)
}

fn write_script(path: &std::path::Path, body: &str) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(body.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
