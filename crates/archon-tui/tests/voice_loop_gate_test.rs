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

/// Read events until a transcript arrives, and return it.
///
/// The loop also emits `VoiceRecording` around every capture (#192) — that is
/// what opens the capture overlay — so a test that wants the transcript has to
/// say so rather than assuming it is the first event on the channel.
async fn next_voice_text(rx: &mut mpsc::Receiver<TuiEvent>) -> String {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TuiEvent::VoiceText(text))) => return text,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("event channel closed before a transcript arrived"),
            Err(_) => panic!("voice_loop emitted no transcript within 3s"),
        }
    }
}

struct StopFailingAudio;

#[async_trait::async_trait]
impl AudioSource for StopFailingAudio {
    async fn start(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<Vec<f32>> {
        anyhow::bail!("stop fixture failure")
    }

    async fn cancel(&self) -> anyhow::Result<()> {
        anyhow::bail!("cancel fixture failure")
    }
}

#[tokio::test]
async fn voice_loop_reports_stop_and_cancel_errors() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(4);
    let pipeline = VoicePipeline::new(
        Arc::new(StopFailingAudio),
        Arc::new(MockStt {
            response: String::new(),
        }),
        0.0,
    );
    let handle = tokio::spawn(voice_loop(trig_rx, evt_tx, pipeline));

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::task::yield_now().await;
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    assert!(
        next_voice_text(&mut evt_rx)
            .await
            .contains("stop fixture failure")
    );

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::task::yield_now().await;
    trig_tx.send(VoiceTrigger::Cancel).await.unwrap();
    assert!(
        next_voice_text(&mut evt_rx)
            .await
            .contains("cancel fixture failure")
    );

    drop(trig_tx);
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("voice loop should exit")
        .expect("voice loop task");
}

#[tokio::test]
async fn voice_loop_toggle_emits_voice_text_event() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(1);

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

    assert_eq!(next_voice_text(&mut evt_rx).await, "hello from mock");

    // Close triggers → loop should exit
    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn voice_loop_second_toggle_starts_new_session() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(1);

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
    let _first = next_voice_text(&mut evt_rx).await;

    // Second cycle — loop must not be dead
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    assert_eq!(next_voice_text(&mut evt_rx).await, "second");

    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn voice_loop_waits_for_event_capacity_without_losing_transcript() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(1);
    evt_tx
        .send(TuiEvent::GenerationStarted)
        .await
        .expect("fill event channel");
    let audio: Arc<dyn AudioSource> = Arc::new(MockAudioSource::with_samples(vec![0.1_f32; 16000]));
    let stt: Arc<dyn SttProvider> = Arc::new(MockStt {
        response: "exact voice transcript".to_string(),
    });
    let pipeline = VoicePipeline::new(audio, stt, 0.0);
    let handle = tokio::spawn(voice_loop(trig_rx, evt_tx, pipeline));

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::task::yield_now().await;
    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    tokio::task::yield_now().await;
    assert!(
        !handle.is_finished(),
        "full event channel must backpressure voice production"
    );

    assert!(matches!(
        evt_rx.recv().await,
        Some(TuiEvent::GenerationStarted)
    ));
    assert_eq!(next_voice_text(&mut evt_rx).await, "exact voice transcript");

    drop(trig_tx);
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("voice loop should exit")
        .expect("voice loop task");
}

/// The hotkey only *asks* for a recording; whether one starts depends on the
/// microphone. So the loop, not the key handler, is what announces that one is
/// running — that announcement is what opens the capture overlay (#192).
#[tokio::test]
async fn voice_loop_announces_the_start_and_end_of_a_recording() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(8);
    let audio: Arc<dyn AudioSource> = Arc::new(MockAudioSource::with_samples(vec![0.1_f32; 16000]));
    let stt: Arc<dyn SttProvider> = Arc::new(MockStt {
        response: "spoken".to_string(),
    });
    let handle = tokio::spawn(voice_loop(
        trig_rx,
        evt_tx,
        VoicePipeline::new(audio, stt, 0.0),
    ));

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    assert!(
        matches!(evt_rx.recv().await, Some(TuiEvent::VoiceRecording(true))),
        "a started recording must be announced before anything else"
    );

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    assert!(
        matches!(evt_rx.recv().await, Some(TuiEvent::VoiceRecording(false))),
        "the end of the recording must be announced before the transcript"
    );
    assert_eq!(next_voice_text(&mut evt_rx).await, "spoken");

    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

/// Cancelling ends the recording too, or the overlay stays on "recording"
/// over a microphone that has already been released.
#[tokio::test]
async fn cancelling_announces_the_end_of_the_recording() {
    let (trig_tx, trig_rx) = mpsc::channel::<VoiceTrigger>(4);
    let (evt_tx, mut evt_rx) = mpsc::channel::<TuiEvent>(8);
    let audio: Arc<dyn AudioSource> = Arc::new(MockAudioSource::with_samples(vec![0.1_f32; 16000]));
    let stt: Arc<dyn SttProvider> = Arc::new(MockStt {
        response: "must not be transcribed".to_string(),
    });
    let handle = tokio::spawn(voice_loop(
        trig_rx,
        evt_tx,
        VoicePipeline::new(audio, stt, 0.0),
    ));

    trig_tx.send(VoiceTrigger::Toggle).await.unwrap();
    assert!(matches!(
        evt_rx.recv().await,
        Some(TuiEvent::VoiceRecording(true))
    ));
    trig_tx.send(VoiceTrigger::Cancel).await.unwrap();
    assert!(matches!(
        evt_rx.recv().await,
        Some(TuiEvent::VoiceRecording(false))
    ));

    let leaked = tokio::time::timeout(Duration::from_millis(300), evt_rx.recv()).await;
    assert!(
        leaked.is_err(),
        "a cancelled recording must not be transcribed: {:?}",
        leaked.ok().flatten()
    );

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

    // Expect NO VoiceText event (silent audio gated by VAD). The recording
    // state events still arrive — a suppressed recording is still a recording
    // that started and ended, and the overlay has to show that.
    let mut started = false;
    let mut ended = false;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(800), evt_rx.recv()).await
    {
        match event {
            TuiEvent::VoiceRecording(true) => started = true,
            TuiEvent::VoiceRecording(false) => ended = true,
            TuiEvent::VoiceText(text) => {
                panic!("VAD should have suppressed silent audio, got: {text}")
            }
            _ => {}
        }
    }
    assert!(started && ended, "the recording was never announced");

    drop(trig_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}
