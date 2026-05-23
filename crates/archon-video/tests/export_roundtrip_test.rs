use archon_video::transcript::{
    TranscriptFormat, TranscriptSegment, export_to_srt, export_to_vtt, parse_transcript,
};

fn segments() -> Vec<TranscriptSegment> {
    vec![
        TranscriptSegment {
            start_ms: 1_000,
            end_ms: 2_500,
            text: "First cue".into(),
            confidence: None,
            speaker: None,
        },
        TranscriptSegment {
            start_ms: 3_000,
            end_ms: 4_000,
            text: "Second cue".into(),
            confidence: None,
            speaker: None,
        },
    ]
}

#[test]
fn vtt_export_round_trips_through_parser() {
    let exported = export_to_vtt(&segments());
    let parsed = parse_transcript(exported.as_bytes(), Some(TranscriptFormat::Vtt)).unwrap();

    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.segments[0].start_ms, 1_000);
    assert_eq!(parsed.segments[1].end_ms, 4_000);
}

#[test]
fn srt_export_round_trips_through_parser() {
    let exported = export_to_srt(&segments());
    let parsed = parse_transcript(exported.as_bytes(), Some(TranscriptFormat::Srt)).unwrap();

    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.segments[0].text, "First cue");
    assert_eq!(parsed.segments[1].start_ms, 3_000);
}
