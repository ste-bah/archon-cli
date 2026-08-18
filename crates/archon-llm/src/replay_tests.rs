//! Tests for the record/replay provider (#189 Phase 5).

use super::*;

use std::sync::atomic::{AtomicU32, Ordering};

use crate::types::{ContentBlockType, Usage};

/// A stand-in for a live provider. Counts its calls, so a test can prove that a
/// replay did not reach it.
struct CountingProvider {
    calls: AtomicU32,
}

impl CountingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }

    fn scripted() -> Vec<StreamEvent> {
        vec![
            StreamEvent::MessageStart {
                id: "msg_live".into(),
                model: "test-model".into(),
                usage: Usage::default(),
            },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            },
            StreamEvent::TextDelta {
                index: 0,
                text: "one ".into(),
            },
            StreamEvent::TextDelta {
                index: 0,
                text: "two".into(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: None,
            },
            StreamEvent::MessageStop,
        ]
    }
}

#[async_trait]
impl LlmProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "test-model".into(),
            display_name: "Test".into(),
            context_window: 1000,
        }]
    }

    async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let events = Self::scripted();
        let (tx, rx) = tokio::sync::mpsc::channel(events.len());
        for event in events {
            let _ = tx.try_send(event);
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: vec![serde_json::json!({"type": "text", "text": "batched"})],
            usage: Usage::default(),
            stop_reason: "end_turn".into(),
        })
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        true
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Cloud
    }
}

fn request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
        ..Default::default()
    }
}

async fn drain(mut rx: Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

fn text_of(events: &[StreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The acceptance criterion, in one test: record against a provider, replay
/// against the directory, and get the same events back — without the provider
/// being called again.
#[tokio::test]
async fn a_recording_replays_identically_and_never_calls_the_provider_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let live = CountingProvider::new();

    let recorder = ReplayProvider::recording(live.clone(), dir.path().to_path_buf());
    let recorded = drain(recorder.stream(request()).await.expect("record")).await;
    assert_eq!(live.calls(), 1);

    let player = ReplayProvider::replaying(dir.path().to_path_buf());
    let replayed = drain(player.stream(request()).await.expect("replay")).await;

    assert_eq!(
        live.calls(),
        1,
        "replay reached the live provider — the whole point is that it cannot"
    );
    assert_eq!(text_of(&replayed), text_of(&recorded));
    assert_eq!(replayed.len(), recorded.len());
}

/// Chunk boundaries are part of the recording. A replay that merged these into
/// one delta would not exercise the agent-loop behaviour that depends on them.
#[tokio::test]
async fn replay_preserves_the_boundaries_the_provider_produced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recorder = ReplayProvider::recording(CountingProvider::new(), dir.path().to_path_buf());
    drain(recorder.stream(request()).await.expect("record")).await;

    let player = ReplayProvider::replaying(dir.path().to_path_buf());
    let replayed = drain(player.stream(request()).await.expect("replay")).await;

    assert_eq!(text_of(&replayed), vec!["one ".to_string(), "two".into()]);
}

/// A miss must fail, loudly, and must not fall through to the network. A silent
/// fallthrough would let a test that stopped exercising its recorded path go on
/// passing.
#[tokio::test]
async fn a_miss_fails_with_a_message_that_says_what_to_do() {
    let dir = tempfile::tempdir().expect("tempdir");
    let player = ReplayProvider::replaying(dir.path().to_path_buf());

    let error = player
        .stream(request())
        .await
        .expect_err("a miss must fail");
    let message = format!("{error}");

    assert!(message.contains("no cassette"), "{message}");
    assert!(message.contains("test-model"), "the model helps: {message}");
    assert!(
        message.contains("never reaches the network"),
        "it must say nothing was sent: {message}"
    );
    assert!(
        message.contains("ARCHON_LLM_REPLAY=record"),
        "it must say how to record it: {message}"
    );
}

#[tokio::test]
async fn a_replay_provider_holds_no_way_to_reach_a_provider() {
    let dir = tempfile::tempdir().expect("tempdir");
    let live = CountingProvider::new();

    // Even handed one, replay mode drops it: there must be no path from a miss
    // to the network, including by mistake.
    let player = ReplayProvider::new(
        ReplayMode::Replay,
        dir.path().to_path_buf(),
        Some(live.clone()),
    );
    let _ = player.stream(request()).await;

    assert_eq!(live.calls(), 0);
}

#[tokio::test]
async fn a_different_request_misses_rather_than_replaying_the_wrong_cassette() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recorder = ReplayProvider::recording(CountingProvider::new(), dir.path().to_path_buf());
    drain(recorder.stream(request()).await.expect("record")).await;

    let mut other = request();
    other.messages = vec![serde_json::json!({"role": "user", "content": "something else"})];
    let player = ReplayProvider::replaying(dir.path().to_path_buf());

    assert!(player.stream(other).await.is_err());
}

#[tokio::test]
async fn a_batched_call_records_and_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let live = CountingProvider::new();
    let recorder = ReplayProvider::recording(live.clone(), dir.path().to_path_buf());
    let recorded = recorder.complete(request()).await.expect("record");

    let player = ReplayProvider::replaying(dir.path().to_path_buf());
    let replayed = player.complete(request()).await.expect("replay");

    assert_eq!(live.calls(), 1);
    assert_eq!(replayed.content, recorded.content);
    assert_eq!(replayed.stop_reason, "end_turn");
}

/// A typo in the mode is a configuration error, and answering it with a live
/// call would be the exact failure this phase removes.
#[tokio::test]
async fn an_unrecognised_mode_refuses_to_call_anything() {
    let dir = tempfile::tempdir().expect("tempdir");
    let live = CountingProvider::new();
    let provider = ReplayProvider::new(
        ReplayMode::Invalid("replya".into()),
        dir.path().to_path_buf(),
        Some(live.clone()),
    );

    let error = provider.stream(request()).await.expect_err("must refuse");
    let message = format!("{error}");

    assert_eq!(live.calls(), 0);
    assert!(message.contains("replya"), "{message}");
    assert!(message.contains("record"), "{message}");
}

/// Recording reaches whatever the real provider reaches. Claiming otherwise
/// would let a recording run past a policy gate that would have stopped the
/// call it is making.
#[test]
fn recording_reports_the_real_provider_data_flow_and_replay_reports_local() {
    let dir = std::path::PathBuf::from(".");
    let recorder = ReplayProvider::recording(CountingProvider::new(), dir.clone());
    assert_eq!(
        recorder.data_flow_classification(),
        DataFlowClassification::Cloud
    );

    let player = ReplayProvider::replaying(dir);
    assert_eq!(
        player.data_flow_classification(),
        DataFlowClassification::Local
    );
}

#[test]
fn the_wrapper_is_inert_unless_the_environment_asks_for_it() {
    // Serialised against the other env-reading test by name: both mutate one
    // process-wide variable.
    let _guard = env_lock();
    unsafe { std::env::remove_var(MODE_ENV) };

    let provider = wrap_if_enabled(CountingProvider::new());

    assert_eq!(
        provider.name(),
        "counting",
        "an unset variable must wrap nothing"
    );
}

#[test]
fn the_wrapper_engages_when_the_environment_asks() {
    let _guard = env_lock();
    unsafe { std::env::set_var(MODE_ENV, "replay") };

    let provider = wrap_if_enabled(CountingProvider::new());
    unsafe { std::env::remove_var(MODE_ENV) };

    assert_eq!(provider.name(), "replay");
}

/// `set_var` is process-wide, so the two tests above cannot overlap.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn the_cassette_directory_defaults_under_the_working_directory() {
    let _guard = env_lock();
    unsafe { std::env::remove_var(DIR_ENV) };

    assert_eq!(
        cassette_dir(),
        std::path::PathBuf::from(".archon/cassettes")
    );
}
