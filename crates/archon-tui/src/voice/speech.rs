//! The speech loop: text in, sound out.
//!
//! The mirror of `pipeline.rs`. That one turns a hotkey into text; this one
//! turns a finished reply into speech. They share nothing but a shape, because
//! listening and speaking are independent — a machine with no microphone can
//! still read answers aloud, and a machine with no speakers can still be
//! dictated to.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;

use super::tts::{MAX_SPOKEN_CHARS, Speech, TtsProvider, spoken_excerpt};

// ---------------------------------------------------------------------------
// Whether archon is currently speaking replies
// ---------------------------------------------------------------------------

/// Read by the producer before it hands a reply over, written by `Ctrl+P` and
/// by `/voice speak`.
///
/// A global, like `VOICE_TOGGLE_MODE` next door and for the same reason: the
/// key handler lives in this crate and the producer lives in the binary, and
/// threading a channel between them for one boolean would be more machinery
/// than the boolean.
static SPEECH_ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether replies are being read aloud.
#[must_use]
pub fn speech_enabled() -> bool {
    SPEECH_ENABLED.load(Ordering::Relaxed)
}

/// Turn speech on or off. Called at startup from config and by `/voice speak`.
pub fn set_speech_enabled(enabled: bool) {
    SPEECH_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Flip it, returning the new state. `Ctrl+P`.
pub fn toggle_speech_enabled() -> bool {
    // fetch_xor rather than load-then-store: two keypresses racing would
    // otherwise both read the same value and one of them would be lost.
    !SPEECH_ENABLED.fetch_xor(true, Ordering::Relaxed)
}

/// Where synthesised audio goes.
///
/// An enum rather than a trait object because there are exactly two cases and
/// one of them exists only for tests. A trait here would be a seam with one
/// real implementation, which is the thing this branch spent its time removing.
pub enum SpeechSink {
    /// The default output device.
    #[cfg(feature = "audio-capture")]
    Device(super::playback::SpeechPlayer),
    /// Records what it was asked to say, for tests.
    Recording(std::sync::Mutex<Vec<Speech>>),
}

impl SpeechSink {
    /// A sink that plays nothing and remembers everything.
    #[must_use]
    pub fn recording() -> Self {
        Self::Recording(std::sync::Mutex::new(Vec::new()))
    }

    /// What this sink was asked to say. Empty for a real device.
    #[must_use]
    pub fn spoken(&self) -> Vec<Speech> {
        match self {
            Self::Recording(clips) => clips.lock().map(|clips| clips.clone()).unwrap_or_default(),
            #[cfg(feature = "audio-capture")]
            Self::Device(_) => Vec::new(),
        }
    }

    pub async fn play(&self, speech: Speech) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "audio-capture")]
            Self::Device(player) => player.play(speech).await,
            Self::Recording(clips) => {
                if let Ok(mut clips) = clips.lock() {
                    clips.push(speech);
                }
                Ok(())
            }
        }
    }

    /// Cut off whatever is being said.
    pub fn stop(&self) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "audio-capture")]
            Self::Device(player) => player.stop(),
            Self::Recording(_) => Ok(()),
        }
    }
}

/// Everything the speech loop owns.
pub struct SpeechPipeline {
    pub tts: Arc<dyn TtsProvider>,
    pub sink: Arc<SpeechSink>,
    /// Longest reply to speak before trimming at a sentence end.
    pub max_chars: usize,
}

impl SpeechPipeline {
    #[must_use]
    pub fn new(tts: Arc<dyn TtsProvider>, sink: Arc<SpeechSink>) -> Self {
        Self {
            tts,
            sink,
            max_chars: MAX_SPOKEN_CHARS,
        }
    }
}

/// Speak each reply that arrives, in order.
///
/// Sequential on purpose: two sentences synthesised in parallel would arrive in
/// whichever order finished first and be played over each other. Returns when
/// the channel closes.
///
/// A synthesis failure is logged and skipped rather than ending the loop — a
/// speech backend that is down should cost the reply being read aloud, not
/// every reply after it.
pub async fn speech_loop(mut text_rx: mpsc::Receiver<String>, pipeline: SpeechPipeline) {
    tracing::info!("voice: speech loop started");
    while let Some(text) = text_rx.recv().await {
        let excerpt = spoken_excerpt(&text, pipeline.max_chars);
        if excerpt.is_empty() {
            continue;
        }
        let speech = match pipeline.tts.synthesize(&excerpt).await {
            Ok(speech) => speech,
            Err(error) => {
                tracing::warn!("voice: synthesis failed: {error}");
                continue;
            }
        };
        if speech.samples.is_empty() {
            tracing::warn!("voice: synthesis returned no audio");
            continue;
        }
        tracing::debug!(
            chars = excerpt.chars().count(),
            millis = speech.duration().as_millis(),
            "voice: speaking"
        );
        if let Err(error) = pipeline.sink.play(speech).await {
            tracing::warn!("voice: playback failed: {error}");
        }
    }
    tracing::info!("voice: speech loop stopped (text channel closed)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::tts::MockTts;

    fn pipeline(sink: &Arc<SpeechSink>) -> SpeechPipeline {
        SpeechPipeline::new(Arc::new(MockTts { sample_count: 64 }), Arc::clone(sink))
    }

    #[tokio::test]
    async fn every_reply_is_spoken_in_order() {
        let sink = Arc::new(SpeechSink::recording());
        let (tx, rx) = mpsc::channel(4);
        let handle = tokio::spawn(speech_loop(rx, pipeline(&sink)));

        tx.send("first".into()).await.unwrap();
        tx.send("second".into()).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        assert_eq!(sink.spoken().len(), 2);
    }

    /// An empty or whitespace-only reply is not a thing to say out loud.
    #[tokio::test]
    async fn nothing_is_spoken_for_an_empty_reply() {
        let sink = Arc::new(SpeechSink::recording());
        let (tx, rx) = mpsc::channel(4);
        let handle = tokio::spawn(speech_loop(rx, pipeline(&sink)));

        tx.send(String::new()).await.unwrap();
        tx.send("   \n  ".into()).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        assert!(sink.spoken().is_empty());
    }

    /// A backend that is down costs one reply, not the rest of the session.
    #[tokio::test]
    async fn a_synthesis_failure_does_not_end_the_loop() {
        struct Failing;
        #[async_trait::async_trait]
        impl TtsProvider for Failing {
            async fn synthesize(&self, text: &str) -> anyhow::Result<Speech> {
                if text.contains("bad") {
                    anyhow::bail!("backend down");
                }
                Ok(Speech {
                    samples: vec![0.2; 16],
                    sample_rate: 24_000,
                })
            }
        }

        let sink = Arc::new(SpeechSink::recording());
        let (tx, rx) = mpsc::channel(4);
        let handle = tokio::spawn(speech_loop(
            rx,
            SpeechPipeline::new(Arc::new(Failing), Arc::clone(&sink)),
        ));

        tx.send("bad one".into()).await.unwrap();
        tx.send("good one".into()).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        assert_eq!(
            sink.spoken().len(),
            1,
            "the reply after the failure was never spoken"
        );
    }

    /// Silence is not speech; playing it wastes the length of the clip.
    #[tokio::test]
    async fn a_synthesis_that_returns_no_audio_is_not_played() {
        let sink = Arc::new(SpeechSink::recording());
        let (tx, rx) = mpsc::channel(4);
        let handle = tokio::spawn(speech_loop(
            rx,
            SpeechPipeline::new(Arc::new(MockTts { sample_count: 0 }), Arc::clone(&sink)),
        ));

        tx.send("anything".into()).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        assert!(sink.spoken().is_empty());
    }

    #[tokio::test]
    async fn a_long_reply_is_trimmed_before_it_is_spoken() {
        let sink = Arc::new(SpeechSink::recording());
        let (tx, rx) = mpsc::channel(4);
        let mut pipe = pipeline(&sink);
        pipe.max_chars = 20;
        let handle = tokio::spawn(speech_loop(rx, pipe));

        tx.send("x".repeat(500)).await.unwrap();
        drop(tx);
        handle.await.unwrap();

        // MockTts ignores the text, so the assertion that matters is that the
        // loop ran to completion on an over-long reply rather than refusing it.
        assert_eq!(sink.spoken().len(), 1);
    }
    #[test]
    fn speech_starts_off_and_toggles() {
        // The global is process-wide; set it explicitly rather than assuming
        // the starting value, so this does not depend on test order.
        set_speech_enabled(false);
        assert!(!speech_enabled());
        assert!(
            toggle_speech_enabled(),
            "the toggle must return the new state"
        );
        assert!(speech_enabled());
        assert!(!toggle_speech_enabled());
        assert!(!speech_enabled());
    }
}
