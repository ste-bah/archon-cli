use super::*;
use serial_test::serial;

fn reset_pending_metric() {
    crate::observability::reset_tui_drain_stall_state_for_tests();
}

#[test]
#[serial(tui_drain_metrics)]
fn pending_bytes_and_high_water_track_enqueue_coalesce_and_dequeue() {
    reset_pending_metric();
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::TextDelta("hello ".into())).unwrap();
    tx.send(TuiEvent::TextDelta("世界".into())).unwrap();

    // Bytes come from this channel's own queue; the high-water mark is a
    // process-wide historical maximum by definition, so it stays on the global
    // and this test stays in the serial group for that reason alone.
    let pending = tx.queued_bytes();
    assert!(pending >= "hello 世界".len());
    assert_eq!(
        crate::observability::tui_event_pending_byte_high_water(),
        pending
    );

    rx.try_recv().unwrap();
    assert_eq!(tx.queued_bytes(), 0);
    assert_eq!(
        crate::observability::tui_event_pending_byte_high_water(),
        pending
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn coalescing_never_retains_allocation_above_frame_bound() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    tx.send(TuiEvent::TextDelta(
        "x".repeat(MAX_COALESCED_CONTENT_BYTES - 1),
    ))
    .unwrap();
    tx.send(TuiEvent::TextDelta("y".into())).unwrap();

    let (len, capacity) = {
        let queue = rx.inner.queue.lock().expect("queue lock");
        let Some(TuiEvent::TextDelta(text)) = queue.front() else {
            panic!("expected text frame");
        };
        (text.len(), text.capacity())
    };
    assert_eq!(len, MAX_COALESCED_CONTENT_BYTES);
    assert!(capacity <= MAX_COALESCED_CONTENT_BYTES);
}

#[test]
fn empty_text_and_thinking_deltas_preserve_event_boundaries() {
    for event in [
        TuiEvent::TextDelta(String::new()),
        TuiEvent::ThinkingDelta(String::new()),
    ] {
        let frames = bounded_content_events(event);
        assert_eq!(frames.len(), 1);
        assert!(matches!(
            &frames[0],
            TuiEvent::TextDelta(text) | TuiEvent::ThinkingDelta(text) if text.is_empty()
        ));
    }
}

#[test]
#[serial(tui_drain_metrics)]
fn closed_receiver_counts_rejected_multiframe_send() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    drop(rx);

    tx.send(TuiEvent::TextDelta(
        "x".repeat(MAX_COALESCED_CONTENT_BYTES + 1),
    ))
    .expect_err("closed receiver must reject multiframe send");

    assert_eq!(
        crate::observability::tui_event_closed_send_failure_count(),
        1
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn file_picker_root_capacity_counts_toward_payload_bound() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let mut root = std::path::PathBuf::from("root");
    root.reserve(MAX_COALESCED_CONTENT_BYTES + 1);
    assert!(root.capacity() > MAX_COALESCED_CONTENT_BYTES);

    tx.send_async(TuiEvent::ShowFilePicker {
        root,
        entries: Vec::new(),
    })
    .await
    .expect_err("oversized retained root path must be rejected");

    assert!(rx.is_empty());
    assert_eq!(
        crate::observability::tui_event_oversized_rejected_count(),
        1
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn file_entry_path_capacity_counts_toward_payload_bound() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let mut path = std::path::PathBuf::from("entry");
    path.reserve(MAX_COALESCED_CONTENT_BYTES + 1);
    assert!(path.capacity() > MAX_COALESCED_CONTENT_BYTES);

    tx.send_async(TuiEvent::ShowFilePicker {
        root: std::path::PathBuf::new(),
        entries: vec![crate::events::FileEntry {
            name: "entry".into(),
            path,
            is_dir: false,
        }],
    })
    .await
    .expect_err("oversized retained entry path must be rejected");

    assert!(rx.is_empty());
    assert_eq!(
        crate::observability::tui_event_oversized_rejected_count(),
        1
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn oversized_tool_summary_is_rejected_and_counted_without_losing_output() {
    reset_pending_metric();
    let output = "tool output".to_string();
    let events = bounded_content_events(TuiEvent::ToolComplete {
        name: "Bash".into(),
        id: "tool-large".into(),
        success: true,
        output: output.clone(),
        transcript_summary: Some("世".repeat(MAX_COALESCED_CONTENT_BYTES)),
    });

    let mut reconstructed = String::new();
    let mut completions = 0;
    for event in events {
        assert!(crate::event_payload_size::heap_bytes(&event) <= MAX_COALESCED_CONTENT_BYTES);
        match event {
            TuiEvent::ToolOutputChunk { chunk, .. } => reconstructed.push_str(&chunk),
            TuiEvent::ToolComplete {
                output,
                transcript_summary,
                ..
            } => {
                completions += 1;
                reconstructed.push_str(&output);
                assert!(transcript_summary.is_none());
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert_eq!(reconstructed, output);
    assert_eq!(completions, 1);
    assert_eq!(
        crate::observability::tui_event_oversized_metadata_rejected_count(),
        1
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn oversized_mcp_state_payload_is_rejected_and_counted() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let event = TuiEvent::ShowMcpManager(vec![crate::events::McpServerEntry {
        name: "server".into(),
        state: "x".repeat(MAX_COALESCED_CONTENT_BYTES),
        tool_count: 0,
        disabled: false,
        tools: Vec::new(),
    }]);

    tx.send(event)
        .expect_err("oversized MCP state must be rejected");

    assert!(rx.is_empty());
    assert_eq!(
        crate::observability::tui_event_oversized_rejected_count(),
        1
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn oversized_vector_payload_is_rejected_and_counted() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let event = TuiEvent::OpenViewRows {
        view_id: crate::events::ViewId::Tasks,
        rows: vec![crate::events::EvidenceRowPayload {
            id: "row".into(),
            title: "title".into(),
            status: "ready".into(),
            detail: "x".repeat(MAX_COALESCED_CONTENT_BYTES),
        }],
    };

    tx.send(event)
        .expect_err("oversized view rows must be rejected");

    assert!(rx.is_empty());
    assert_eq!(
        crate::observability::tui_event_oversized_rejected_count(),
        1
    );
}

#[test]
#[serial(tui_drain_metrics)]
fn oversized_unframed_payload_is_rejected_and_counted() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let event = TuiEvent::Error("x".repeat(MAX_COALESCED_CONTENT_BYTES + 1));

    let error = tx
        .send(event)
        .expect_err("oversized error must be rejected");

    assert!(
        matches!(error.0, TuiEvent::Error(message) if message.len() == MAX_COALESCED_CONTENT_BYTES + 1)
    );
    assert!(rx.is_empty());
    assert_eq!(
        crate::observability::tui_event_oversized_rejected_count(),
        1
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn sender_frames_direct_oversized_tool_output_chunk() {
    let expected = format!("{}世界", "x".repeat(MAX_COALESCED_CONTENT_BYTES));
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::ToolOutputChunk {
        id: "tool-large".into(),
        chunk: expected.clone(),
    })
    .unwrap();

    let mut reconstructed = String::new();
    while let Ok(event) = rx.try_recv() {
        let TuiEvent::ToolOutputChunk { id, chunk } = event else {
            panic!("unexpected event: {event:?}");
        };
        assert_eq!(id, "tool-large");
        assert!(chunk.len() <= MAX_COALESCED_CONTENT_BYTES);
        reconstructed.push_str(&chunk);
    }
    assert_eq!(reconstructed, expected);
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn sender_frames_oversized_text_before_queue_admission() {
    let expected = format!("{}世界", "x".repeat(MAX_COALESCED_CONTENT_BYTES));
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);

    tx.send(TuiEvent::TextDelta(expected.clone())).unwrap();

    let mut reconstructed = String::new();
    while let Ok(event) = rx.try_recv() {
        let TuiEvent::TextDelta(chunk) = event else {
            panic!("unexpected event: {event:?}");
        };
        assert!(chunk.len() <= MAX_COALESCED_CONTENT_BYTES);
        reconstructed.push_str(&chunk);
    }
    assert_eq!(reconstructed, expected);
}

#[test]
#[serial(tui_drain_metrics)]
fn synchronous_multiframe_rejection_is_atomic_and_counted() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let event = TuiEvent::TextDelta("x".repeat(MAX_COALESCED_CONTENT_BYTES * 3));

    tx.send(event)
        .expect_err("three frames must not fit capacity two");

    assert!(rx.is_empty());
    assert_eq!(crate::observability::tui_event_full_send_failure_count(), 1);
}

#[test]
#[serial(tui_drain_metrics)]
fn synchronous_utf8_frame_count_preserves_atomic_capacity_bound() {
    reset_pending_metric();
    let (tx, rx) = bounded_tui_event_channel_with_capacity(2);
    let event = TuiEvent::ToolOutputChunk {
        id: "i".into(),
        chunk: "é".repeat(MAX_COALESCED_CONTENT_BYTES - 1),
    };

    tx.send(event)
        .expect_err("UTF-8 boundary adjustment requires three frames");

    assert!(rx.is_empty());
    assert_eq!(crate::observability::tui_event_full_send_failure_count(), 1);
}

#[test]
fn tool_output_with_insufficient_utf8_frame_budget_is_rejected_as_single_oversized_event() {
    let id = "i".repeat(MAX_COALESCED_CONTENT_BYTES - 1);
    let frames = crate::event_framing::ContentFrames::new(TuiEvent::ToolOutputChunk {
        id,
        chunk: "é".into(),
    });

    assert!(matches!(
        frames,
        crate::event_framing::ContentFrames::Single(Some(TuiEvent::ToolOutputChunk { .. }))
    ));
}

#[test]
fn async_content_frames_do_not_prebuild_chunk_vector() {
    let expected = "x".repeat(MAX_COALESCED_CONTENT_BYTES * 3);
    let frames = crate::event_framing::ContentFrames::new(TuiEvent::TextDelta(expected.clone()));

    let crate::event_framing::ContentFrames::Text {
        text, next_offset, ..
    } = frames
    else {
        panic!("expected lazy text frame state");
    };
    assert_eq!(text, expected);
    assert_eq!(next_offset, 0);
}

#[test]
fn oversized_text_and_thinking_deltas_split_without_losing_utf8() {
    for event in [
        TuiEvent::TextDelta(format!("{}世界", "x".repeat(MAX_COALESCED_CONTENT_BYTES))),
        TuiEvent::ThinkingDelta(format!("{}世界", "y".repeat(MAX_COALESCED_CONTENT_BYTES))),
    ] {
        let expected = match &event {
            TuiEvent::TextDelta(text) | TuiEvent::ThinkingDelta(text) => text.clone(),
            _ => unreachable!(),
        };
        let mut reconstructed = String::new();
        for frame in bounded_content_events(event) {
            match frame {
                TuiEvent::TextDelta(chunk) | TuiEvent::ThinkingDelta(chunk) => {
                    assert!(chunk.len() <= MAX_COALESCED_CONTENT_BYTES);
                    assert!(chunk.capacity() <= MAX_COALESCED_CONTENT_BYTES);
                    reconstructed.push_str(&chunk);
                }
                frame => panic!("unexpected frame: {frame:?}"),
            }
        }
        assert_eq!(reconstructed, expected);
    }
}

#[test]
fn oversized_tool_output_is_split_into_bounded_utf8_chunks() {
    let output = format!("{}世界", "0123456789abcdef\n".repeat(5_000));
    let events = bounded_content_events(TuiEvent::ToolComplete {
        name: "Bash".into(),
        id: "tool-large".into(),
        success: true,
        output: output.clone(),
        transcript_summary: None,
    });

    let mut reconstructed = String::new();
    let mut completions = 0;
    for event in events {
        match event {
            TuiEvent::ToolOutputChunk { id, chunk } => {
                assert_eq!(id, "tool-large");
                assert!(chunk.len() <= MAX_COALESCED_CONTENT_BYTES);
                reconstructed.push_str(&chunk);
            }
            TuiEvent::ToolComplete { output, .. } => {
                completions += 1;
                assert!(output.is_empty());
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert_eq!(reconstructed, output);
    assert_eq!(completions, 1);
}
