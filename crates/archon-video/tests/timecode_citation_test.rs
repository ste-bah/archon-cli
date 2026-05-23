use archon_docs::answer::{Citation, TimecodeMs, format_citation};

fn citation() -> Citation {
    Citation {
        chunk_id: "chunk-video-1".into(),
        document_id: "doc-video".into(),
        page_start: 0,
        page_end: 0,
        snippet: "The slide describes match scoring thresholds.".into(),
    }
}

#[test]
fn video_citation_renders_timecode() {
    let rendered = format_citation(1, &citation(), 0.91, Some(TimecodeMs { start_ms: 760_000 }));

    assert!(rendered.contains("video@12:40"));
    assert!(rendered.contains("score 0.91"));
}

#[test]
fn non_video_citation_keeps_page_format() {
    let rendered = format_citation(1, &citation(), 0.91, None);

    assert!(rendered.contains("(pages 0-0, score 0.91):"));
    assert!(!rendered.contains("video@"));
}
