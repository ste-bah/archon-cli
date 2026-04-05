//! Integration test for TASK-WIRE-007: real voice event loop.
//!
//! Gate 1 test-first — written BEFORE voice_loop implementation.
//!
//! Asserts end-to-end pipeline: trigger (Toggle start) → record audio →
//! trigger (Toggle stop) → encode WAV → transcribe → emit TuiEvent::VoiceText.
//!
//! Uses MockAudioSource + MockStt to run the loop without cpal/ALSA/network.

use std::sync::Arc;
use std::time::Duration;

use archon_tui::app::TuiEvent;
use archon_tui::voice::pipeline::{
    AudioSource, MockAudioSource, VoicePipeline, VoiceTrigger, voice_loop,
};
use archon_tui::voice::stt::{MockStt, SttProvider};
use tokio::sync::mpsc;

#[tokio::test]
async fn voice_loop_toggle_emits_voice_text_event() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(4);

    let audio: Arc<dyn AudioSource> = Arc::new(MockAudioSource::with_samples(vec![0.1_f32; 16000])); // 1 sec
    let stt: Arc<dyn SttProvider> = Arc::new(MockStt {
        response: "hello from mock".to_string(),
    });
    let pipeline = VoicePipeline::new(audio, stt, 0.0);

    // Spawn the loop
    let handle = tokio::spawn(voice_loop(trig_rx, evt_tx, pipeline));

    // Toggle on → start capture
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    // Small delay so the loop registers "recording"
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Toggle off → stop, encode, transcribe, emit
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();

    // Expect VoiceText event within a timeout
    let evt = tokio::time::timeout(Duration::from_secs(3), evt_rx.recv())
        .await
        .expect("voice_loop did not emit an event within 3s")
        .expect("event channel closed unexpectedly");

    match evt {
        TuiEvent::VoiceText(text) => assert_eq!(text, "hello from mock"),
        other => panic!("expected TuiEvent::VoiceText, got {other:?}"),
    }

    // Close triggers → loop should exit
    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn voice_loop_second_toggle_starts_new_session() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(4);

    let audio: Arc<dyn AudioSource> = Arc::new(MockAudioSource::with_samples(vec![0.2_f32; 8000]));
    let stt: Arc<dyn SttProvider> = Arc::new(MockStt {
        response: "second".to_string(),
    });
    let pipeline = VoicePipeline::new(audio, stt, 0.0);
    let handle = tokio::spawn(voice_loop(trig_rx, evt_tx, pipeline));

    // First full toggle cycle
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(3), evt_rx.recv())
        .await
        .expect("first cycle timed out")
        .unwrap();
    assert!(matches!(first, TuiEvent::VoiceText(_)));

    // Second cycle — loop must not be dead
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    let second = tokio::time::timeout(Duration::from_secs(3), evt_rx.recv())
        .await
        .expect("second cycle timed out")
        .unwrap();
    match second {
        TuiEvent::VoiceText(t) => assert_eq!(t, "second"),
        other => panic!("expected VoiceText, got {other:?}"),
    }

    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn voice_loop_vad_rejects_silent_audio() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(4);

    // Silent samples (below VAD threshold)
    let audio: Arc<dyn AudioSource> =
        Arc::new(MockAudioSource::with_samples(vec![0.001_f32; 16000]));
    let stt: Arc<dyn SttProvider> = Arc::new(MockStt {
        response: "should-not-be-emitted".to_string(),
    });
    // High VAD threshold → silence detected
    let pipeline = VoicePipeline::new(audio, stt, 0.5);
    let handle = tokio::spawn(voice_loop(trig_rx, evt_tx, pipeline));

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();

    // Expect NO VoiceText event (silent audio gated by VAD)
    let result = tokio::time::timeout(Duration::from_millis(800), evt_rx.recv()).await;
    assert!(
        result.is_err(),
        "VAD should have suppressed silent audio, got event: {:?}",
        result.ok().flatten()
    );

    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}
