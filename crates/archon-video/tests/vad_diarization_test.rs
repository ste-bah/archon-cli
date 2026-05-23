use archon_policy::EffectivePolicy;
use async_trait::async_trait;
use cozo::{DbInstance, ScriptMutability};

use archon_video::asr::{
    DiarizationProvider, MockAsrAdapter, MockDiarizerProvider, NullDiarizerProvider,
    apply_diarization, enforce_monotonic_boundaries,
};
use archon_video::errors::VideoError;
use archon_video::ingest::{IngestOpts, ingest_video_with_providers};
use archon_video::schema::create_video_schema;
use archon_video::transcript::TranscriptSegment;

fn segment(start_ms: u64, end_ms: u64, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        start_ms,
        end_ms,
        text: text.into(),
        confidence: Some(0.9),
        speaker: None,
    }
}

#[test]
fn vad_post_processing_enforces_monotonic_non_overlapping_segments() {
    let mut segments = vec![
        segment(1_000, 2_000, "first"),
        segment(1_500, 1_600, "overlap"),
        segment(1_550, 1_550, "zero duration"),
    ];

    enforce_monotonic_boundaries(&mut segments);

    assert_eq!(segments[1].start_ms, segments[0].end_ms);
    assert!(segments[1].end_ms > segments[1].start_ms);
    assert_eq!(segments[2].start_ms, segments[1].end_ms);
    assert!(segments[2].end_ms > segments[2].start_ms);
}

#[tokio::test]
async fn mock_diarizer_populates_alternating_speakers() {
    let provider = MockDiarizerProvider;
    let (segments, warnings) = apply_diarization(
        vec![segment(0, 500, "a"), segment(500, 900, "b")],
        b"",
        &provider,
    )
    .await;

    assert!(warnings.is_empty());
    assert_eq!(segments[0].speaker.as_deref(), Some("SPEAKER_A"));
    assert_eq!(segments[1].speaker.as_deref(), Some("SPEAKER_B"));
}

#[tokio::test]
async fn null_diarizer_leaves_speakers_unset() {
    let provider = NullDiarizerProvider;
    let (segments, warnings) = apply_diarization(vec![segment(0, 500, "a")], b"", &provider).await;

    assert!(warnings.is_empty());
    assert!(segments[0].speaker.is_none());
}

struct FailingDiarizer;

#[async_trait]
impl DiarizationProvider for FailingDiarizer {
    async fn attribute_speakers(
        &self,
        _segments: Vec<TranscriptSegment>,
        _audio_bytes: &[u8],
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        Err(VideoError::AsrProviderUnavailable {
            message: "diarizer missing".into(),
        })
    }

    fn provider_name(&self) -> &str {
        "failing"
    }
}

#[tokio::test]
async fn diarization_failure_warns_and_preserves_segments() {
    let provider = FailingDiarizer;
    let (segments, warnings) = apply_diarization(vec![segment(0, 500, "a")], b"", &provider).await;

    assert_eq!(segments[0].text, "a");
    assert!(segments[0].speaker.is_none());
    assert_eq!(warnings.len(), 1);
}

#[tokio::test]
async fn ingest_with_mock_diarizer_persists_speaker_labels() {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;
    policy.video.asr.diarization = true;
    let asr = MockAsrAdapter {
        segments: vec![segment(0, 500, "a"), segment(500, 900, "b")],
    };
    let diarizer = MockDiarizerProvider;

    let result = ingest_video_with_providers(
        IngestOpts {
            source: "diarized_fixture.mp4".into(),
            transcript_path: None,
            metadata_only: false,
            frames_mode: None,
            asr_provider: None,
            vlm: false,
            yes: true,
        },
        &policy,
        &db,
        Some(&asr),
        Some(&diarizer),
    )
    .await
    .unwrap();

    assert_eq!(result.chunk_count, 2);
    let rows = db
        .run_script(
            "?[speaker] := *video_transcript_segments{speaker}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    let speakers: Vec<_> = rows
        .rows
        .iter()
        .filter_map(|row| row[0].get_str())
        .collect();
    assert!(speakers.contains(&"SPEAKER_A"));
    assert!(speakers.contains(&"SPEAKER_B"));
}
