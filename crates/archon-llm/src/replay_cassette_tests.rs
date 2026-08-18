//! Tests for the cassette file format (#189 Phase 5).

use super::*;

use crate::types::ContentBlockType;

fn events() -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "msg_1".into(),
            model: "claude-sonnet-4-6".into(),
            usage: Usage {
                input_tokens: 12,
                input_tokens_available: true,
                ..Usage::default()
            },
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: "Hel".into(),
        },
        StreamEvent::TextDelta {
            index: 0,
            text: "lo".into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                output_tokens: 3,
                output_tokens_available: true,
                ..Usage::default()
            }),
        },
        StreamEvent::MessageStop,
    ]
}

fn cassette() -> Cassette {
    Cassette {
        digest: "abc123".into(),
        provider: "anthropic".into(),
        model: "claude-sonnet-4-6".into(),
        canonical_request: r#"{"model":"claude-sonnet-4-6"}"#.into(),
        events: events(),
        response: None,
    }
}

#[test]
fn a_cassette_round_trips_through_disk_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let saved = cassette().save(dir.path()).expect("save");

    let loaded = Cassette::load(&saved).expect("load");

    assert_eq!(loaded.digest, "abc123");
    assert_eq!(loaded.events.len(), events().len());
    assert_eq!(loaded.model, "claude-sonnet-4-6");
}

/// The reason boundaries are recorded at all: a lot of agent-loop behaviour is
/// sensitive to how a response is chunked, and a replay that delivered one
/// large delta would not exercise any of it.
#[tokio::test]
async fn replay_reproduces_the_original_chunk_boundaries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let saved = cassette().save(dir.path()).expect("save");
    let loaded = Cassette::load(&saved).expect("load");

    let mut rx = loaded.replay_events();
    let mut deltas = Vec::new();
    while let Some(event) = rx.recv().await {
        if let StreamEvent::TextDelta { text, .. } = event {
            deltas.push(text);
        }
    }

    assert_eq!(
        deltas,
        vec!["Hel".to_string(), "lo".to_string()],
        "the two deltas were merged into one"
    );
}

#[tokio::test]
async fn the_full_event_sequence_survives_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let saved = cassette().save(dir.path()).expect("save");

    let mut rx = Cassette::load(&saved).expect("load").replay_events();
    let mut kinds = Vec::new();
    while let Some(event) = rx.recv().await {
        kinds.push(std::mem::discriminant(&event));
    }
    let original: Vec<_> = events().iter().map(std::mem::discriminant).collect();

    assert_eq!(kinds, original);
}

/// An agent may call `complete` where the recording was made through `stream`.
/// Refusing would make a cassette depend on the caller rather than the request.
#[tokio::test]
async fn a_streamed_recording_can_answer_a_complete_call() {
    let response = cassette().as_response().await.expect("response");

    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!(
        response.content,
        vec![serde_json::json!({"type": "text", "text": "Hello"})]
    );
    assert_eq!(response.usage.input_tokens, 12);
    assert_eq!(response.usage.output_tokens, 3);
}

#[tokio::test]
async fn a_recorded_complete_response_is_returned_verbatim() {
    let mut recorded = cassette();
    recorded.events.clear();
    recorded.response = Some(RecordedResponse {
        content: vec![serde_json::json!({"type": "text", "text": "batched"})],
        usage: Usage::default(),
        stop_reason: "max_tokens".into(),
    });

    let response = recorded.as_response().await.expect("response");

    assert_eq!(response.stop_reason, "max_tokens");
}

/// An empty cassette is a bug in whatever wrote it, and saying so beats
/// answering with an empty response that reads like a model returning nothing.
#[tokio::test]
async fn an_empty_cassette_is_an_error_rather_than_an_empty_answer() {
    let mut empty = cassette();
    empty.events.clear();

    let error = empty.as_response().await.expect_err("empty is an error");

    assert!(format!("{error}").contains("neither"), "{error}");
}

#[test]
fn a_corrupt_cassette_names_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("broken.json");
    std::fs::write(&path, "{ not json").expect("write");

    let error = Cassette::load(&path).expect_err("corrupt");

    assert!(error.contains("broken.json"), "{error}");
}

#[test]
fn saving_creates_the_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = dir.path().join("does").join("not").join("exist");

    let saved = cassette().save(&nested).expect("save creates the path");

    assert!(saved.exists());
}
