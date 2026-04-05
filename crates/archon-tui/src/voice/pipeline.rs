//! Voice capture → STT event pipeline.
//!
//! The [`voice_loop`] task converts `Toggle` triggers from the TUI hotkey
//! handler into [`TuiEvent::VoiceText`] events by:
//!
//! 1. On first toggle: start audio capture via [`AudioSource`].
//! 2. On second toggle: stop capture, run VAD, encode WAV, transcribe, emit.
//! 3. Loop forever until the trigger channel is closed.
//!
//! Tests use [`MockAudioSource`] + `MockStt` to exercise the loop without
//! cpal/ALSA or network access.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};

// ---------------------------------------------------------------------------
// Global trigger sender (bridges TUI key handler → voice_loop without
// changing run_tui's public signature)
// ---------------------------------------------------------------------------

static VOICE_TRIGGER_TX: OnceLock<mpsc::Sender<VoiceTrigger>> = OnceLock::new();

/// Install the voice trigger sender. Only the first call takes effect.
pub fn install_trigger_sender(tx: mpsc::Sender<VoiceTrigger>) {
    let _ = VOICE_TRIGGER_TX.set(tx);
}

/// Fire a voice trigger (does nothing if no voice loop is active).
pub fn fire_trigger(trigger: VoiceTrigger) {
    if let Some(tx) = VOICE_TRIGGER_TX.get() {
        let _ = tx.try_send(trigger);
    }
}

use crate::app::TuiEvent;
use crate::voice::capture::{AudioCapture, VoiceActivityDetector};
use crate::voice::stt::SttProvider;

// ---------------------------------------------------------------------------
// Trigger
// ---------------------------------------------------------------------------

/// Commands the hotkey layer sends to the voice loop.
#[derive(Debug, Clone)]
pub enum VoiceTrigger {
    /// Toggle recording (start if idle, stop+transcribe if recording).
    Toggle,
    /// Cancel the current recording (if any) without transcribing.
    Cancel,
}

// ---------------------------------------------------------------------------
// AudioSource trait
// ---------------------------------------------------------------------------

/// A source of PCM f32 audio samples at 16 kHz mono.
///
/// Real implementations use cpal; tests use [`MockAudioSource`].
#[async_trait]
pub trait AudioSource: Send + Sync {
    /// Begin capturing audio. Previously captured samples (if any) are dropped.
    async fn start(&self) -> anyhow::Result<()>;
    /// Stop capture and return the accumulated f32 samples (16 kHz mono).
    async fn stop(&self) -> anyhow::Result<Vec<f32>>;
    /// Cancel a recording, discarding its samples.
    async fn cancel(&self) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// MockAudioSource (for tests + smoke runs without a microphone)
// ---------------------------------------------------------------------------

/// Deterministic AudioSource that always returns a pre-seeded sample buffer
/// from [`stop`]. Safe to use in unit tests and in environments with no
/// audio device (e.g. CI, WSL2 without libasound).
pub struct MockAudioSource {
    samples: Mutex<Vec<f32>>,
    started: Mutex<bool>,
}

impl MockAudioSource {
    pub fn with_samples(samples: Vec<f32>) -> Self {
        Self {
            samples: Mutex::new(samples),
            started: Mutex::new(false),
        }
    }
}

#[async_trait]
impl AudioSource for MockAudioSource {
    async fn start(&self) -> anyhow::Result<()> {
        *self.started.lock().await = true;
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<Vec<f32>> {
        let mut started = self.started.lock().await;
        if !*started {
            anyhow::bail!("MockAudioSource: stop called before start");
        }
        *started = false;
        // Return a clone of the seeded samples — tests can toggle multiple
        // times and get the same audio each recording.
        Ok(self.samples.lock().await.clone())
    }

    async fn cancel(&self) -> anyhow::Result<()> {
        *self.started.lock().await = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// VoicePipeline + voice_loop
// ---------------------------------------------------------------------------

/// All runtime components the voice loop owns.
pub struct VoicePipeline {
    pub audio: Arc<dyn AudioSource>,
    pub stt: Arc<dyn SttProvider>,
    pub capture: AudioCapture,
    pub vad: VoiceActivityDetector,
}

impl VoicePipeline {
    pub fn new(
        audio: Arc<dyn AudioSource>,
        stt: Arc<dyn SttProvider>,
        vad_threshold: f32,
    ) -> Self {
        Self {
            audio,
            stt,
            capture: AudioCapture::new(),
            vad: VoiceActivityDetector::new(vad_threshold),
        }
    }
}

/// The voice event loop.
///
/// Receives [`VoiceTrigger`]s, drives audio capture, transcribes via STT,
/// and emits [`TuiEvent::VoiceText`] to the TUI event channel. Returns when
/// `trigger_rx` is closed (all senders dropped).
pub async fn voice_loop(
    mut trigger_rx: mpsc::Receiver<VoiceTrigger>,
    tui_event_tx: mpsc::Sender<TuiEvent>,
    pipeline: VoicePipeline,
) {
    tracing::info!("voice: pipeline started");
    let mut recording = false;

    while let Some(trigger) = trigger_rx.recv().await {
        match trigger {
            VoiceTrigger::Toggle if !recording => {
                if let Err(e) = pipeline.audio.start().await {
                    tracing::warn!("voice: start failed: {e}");
                    let _ = tui_event_tx
                        .send(TuiEvent::VoiceText(format!("[voice error: {e}]")))
                        .await;
                    continue;
                }
                recording = true;
                tracing::info!("voice: recording");
            }
            VoiceTrigger::Toggle => {
                // recording = true → stop, transcribe, emit
                recording = false;
                let samples = match pipeline.audio.stop().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("voice: stop failed: {e}");
                        continue;
                    }
                };
                if !pipeline.vad.is_speech(&samples) {
                    tracing::info!(
                        "voice: VAD suppressed {} samples (below threshold)",
                        samples.len()
                    );
                    continue;
                }
                let wav = pipeline.capture.encode_to_wav(&samples);
                match pipeline.stt.transcribe(&wav).await {
                    Ok(text) => {
                        tracing::info!("voice: transcribed {} chars", text.len());
                        if tui_event_tx.send(TuiEvent::VoiceText(text)).await.is_err() {
                            tracing::warn!("voice: tui event channel closed; exiting loop");
                            return;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("voice: transcribe failed: {e}");
                        let _ = tui_event_tx
                            .send(TuiEvent::VoiceText(format!("[stt error: {e}]")))
                            .await;
                    }
                }
            }
            VoiceTrigger::Cancel => {
                if recording {
                    recording = false;
                    if let Err(e) = pipeline.audio.cancel().await {
                        tracing::warn!("voice: cancel failed: {e}");
                    } else {
                        tracing::info!("voice: recording cancelled");
                    }
                }
            }
        }
    }
    tracing::info!("voice: pipeline stopped (trigger channel closed)");
}
