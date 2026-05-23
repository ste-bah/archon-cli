use archon_video::transcript::{
    TranscriptFormat, export_to_srt, export_to_txt, export_to_vtt, parse_transcript,
};

#[test]
fn parses_supported_transcript_formats() {
    let cases = [
        (
            include_bytes!("fixtures/mini_lecture.vtt").as_slice(),
            Some(TranscriptFormat::Vtt),
        ),
        (
            include_bytes!("fixtures/mini_lecture.srt").as_slice(),
            Some(TranscriptFormat::Srt),
        ),
        (
            br#"<tt><body><p begin="00:00:01.000" end="00:00:02.000">One</p><p begin="00:00:02.000" end="00:00:04.000">Two</p></body></tt>"#.as_slice(),
            Some(TranscriptFormat::Ttml),
        ),
        (
            br#"[{"start":1.0,"end":2.0,"text":"One"},{"start_time":"00:00:02.000","end_time":"00:00:04.000","text":"Two"}]"#.as_slice(),
            Some(TranscriptFormat::Json),
        ),
        (
            b"[00:00:01] One\n[00:00:02] Two".as_slice(),
            Some(TranscriptFormat::PlainText),
        ),
    ];

    for (content, format) in cases {
        let parsed = parse_transcript(content, format).unwrap();
        assert!(!parsed.segments.is_empty());
        assert!(
            parsed
                .segments
                .iter()
                .all(|segment| segment.end_ms > segment.start_ms)
        );
    }
}

#[test]
fn malformed_vtt_warns_and_keeps_valid_segments() {
    let parsed = parse_transcript(
        include_bytes!("fixtures/malformed_transcript.vtt"),
        Some(TranscriptFormat::Vtt),
    )
    .unwrap();

    assert_eq!(parsed.segments.len(), 2);
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.contains("malformed"))
    );
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.contains("overlapping"))
    );
}

#[test]
fn vtt_round_trip_preserves_segments() {
    let parsed = parse_transcript(
        include_bytes!("fixtures/mini_lecture.vtt"),
        Some(TranscriptFormat::Vtt),
    )
    .unwrap();
    let exported = export_to_vtt(&parsed.segments);
    let reparsed = parse_transcript(exported.as_bytes(), Some(TranscriptFormat::Vtt)).unwrap();

    assert_eq!(reparsed.segments, parsed.segments);
}

#[test]
fn exports_srt_vtt_and_plain_text() {
    let parsed = parse_transcript(
        include_bytes!("fixtures/mini_lecture.vtt"),
        Some(TranscriptFormat::Vtt),
    )
    .unwrap();

    let srt = export_to_srt(&parsed.segments);
    assert!(srt.contains("1\n00:00:01,000 --> 00:00:03,000"));
    assert!(export_to_vtt(&parsed.segments).starts_with("WEBVTT\n\n"));
    assert!(export_to_txt(&parsed.segments).contains("architecture review"));
}
