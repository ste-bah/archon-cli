//! The microphone, behind the [`AudioSource`] trait.
//!
//! Until this module existed the production wiring built a `MockAudioSource`
//! seeded with a second of zeroes — in *both* branches of a check that logged
//! "real audio device detected". Voice input therefore never failed and never
//! worked: it recorded silence, the VAD discarded it, and nothing said so.
//!
//! ## Why a thread
//!
//! `cpal::Stream` is not `Send` on every backend, and [`AudioSource`] is an
//! async trait shared across tasks. So the stream never leaves the thread that
//! built it: [`CpalAudioSource`] owns a command channel, a dedicated OS thread
//! owns the stream, and each command carries a `oneshot` for its reply.
//!
//! ## Why the buffer is at device rate
//!
//! Samples accumulate mono at whatever rate the device chose and are resampled
//! to 16 kHz once, in `stop`. Resampling per callback would need interpolation
//! state carried across buffer boundaries; doing it once needs none, and the
//! arithmetic lives in `capture.rs` where it is tested without a soundcard.

use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use tokio::sync::{mpsc, oneshot};

use super::capture::{mix_frames_to_mono, resample_linear, rms};
use super::pipeline::AudioSource;

/// What the STT providers expect.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// A recording longer than this is a stuck hotkey, not dictation.
///
/// At 48 kHz mono that ceiling is about 57 MB of `f32`, which is the point of
/// having one: the buffer grows for as long as the user holds the key, and an
/// unbounded one would eventually take the process down.
const MAX_CAPTURE_SECONDS: usize = 300;

/// How often the level meter reports, in updates per second.
///
/// The audio callback fires every few milliseconds. Forwarding one event per
/// callback would swamp the TUI event channel to redraw a bar chart; 20 Hz is
/// already faster than a terminal usefully animates.
const LEVEL_UPDATES_PER_SECOND: u32 = 20;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

enum Command {
    Start(oneshot::Sender<anyhow::Result<()>>),
    Stop(oneshot::Sender<anyhow::Result<Vec<f32>>>),
    Cancel(oneshot::Sender<anyhow::Result<()>>),
}

/// Samples captured so far, shared with the audio callback.
struct CaptureBuffer {
    samples: Vec<f32>,
    max_samples: usize,
    /// Set when the ceiling was hit, so `stop` can say the tail was dropped
    /// rather than quietly returning a truncated recording.
    truncated: bool,
}

impl CaptureBuffer {
    fn new(sample_rate: u32) -> Self {
        Self {
            samples: Vec::new(),
            max_samples: sample_rate as usize * MAX_CAPTURE_SECONDS,
            truncated: false,
        }
    }

    fn push(&mut self, mono: &[f32]) {
        let room = self.max_samples.saturating_sub(self.samples.len());
        if room == 0 {
            self.truncated = true;
            return;
        }
        if mono.len() > room {
            self.truncated = true;
            self.samples.extend_from_slice(&mono[..room]);
            return;
        }
        self.samples.extend_from_slice(mono);
    }
}

/// A live stream and the buffer it is filling.
struct ActiveStream {
    /// Held only to keep the stream alive; dropping it stops capture.
    _stream: cpal::Stream,
    buffer: Arc<Mutex<CaptureBuffer>>,
    sample_rate: u32,
}

// ---------------------------------------------------------------------------
// CpalAudioSource
// ---------------------------------------------------------------------------

/// Records from a real input device.
pub struct CpalAudioSource {
    /// `None` only during drop, so the thread's channel closes before the join.
    commands: Option<sync_mpsc::Sender<Command>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CpalAudioSource {
    /// Open a capture thread for `device`, which is a device name or
    /// `"default"`.
    ///
    /// The device is resolved here rather than at the first `start` so a
    /// missing microphone is reported at startup, where the caller can decline
    /// to run a voice pipeline, instead of at the first hotkey press.
    ///
    /// `level_tx` receives an RMS level roughly [`LEVEL_UPDATES_PER_SECOND`]
    /// times a second while recording. Levels are dropped rather than queued
    /// when the receiver is behind: blocking an audio callback would glitch the
    /// recording to keep a meter smooth, which is the wrong trade.
    pub fn new(device: &str, level_tx: Option<mpsc::Sender<f32>>) -> anyhow::Result<Self> {
        // Fail now if there is nothing to record from.
        let name = describe_input_device(device)?;
        tracing::info!("voice: capture device {name}");

        let (tx, rx) = sync_mpsc::channel();
        let preference = device.to_string();
        let thread = std::thread::Builder::new()
            .name("archon-voice-capture".to_string())
            .spawn(move || capture_thread(&rx, &preference, level_tx.as_ref()))
            .context("could not start the voice capture thread")?;

        Ok(Self {
            commands: Some(tx),
            thread: Some(thread),
        })
    }

    fn send(&self, command: Command) -> anyhow::Result<()> {
        let sender = self
            .commands
            .as_ref()
            .ok_or_else(|| anyhow!("the voice capture thread is shutting down"))?;
        sender
            .send(command)
            .map_err(|_| anyhow!("the voice capture thread has stopped"))
    }
}

impl Drop for CpalAudioSource {
    fn drop(&mut self) {
        // Closing the channel is what ends the thread's `recv` loop, so the
        // sender must go before the join or this deadlocks.
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[async_trait]
impl AudioSource for CpalAudioSource {
    async fn start(&self) -> anyhow::Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send(Command::Start(reply_tx))?;
        reply_rx.await.context("the voice capture thread died")?
    }

    async fn stop(&self) -> anyhow::Result<Vec<f32>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send(Command::Stop(reply_tx))?;
        reply_rx.await.context("the voice capture thread died")?
    }

    async fn cancel(&self) -> anyhow::Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send(Command::Cancel(reply_tx))?;
        reply_rx.await.context("the voice capture thread died")?
    }
}

// ---------------------------------------------------------------------------
// The capture thread
// ---------------------------------------------------------------------------

fn capture_thread(
    commands: &sync_mpsc::Receiver<Command>,
    preference: &str,
    level_tx: Option<&mpsc::Sender<f32>>,
) {
    let mut active: Option<ActiveStream> = None;

    while let Ok(command) = commands.recv() {
        match command {
            Command::Start(reply) => {
                // A second start without a stop would leak the first stream and
                // silently discard what it captured.
                if active.is_some() {
                    let _ = reply.send(Err(anyhow!("already recording")));
                    continue;
                }
                let outcome = match start_stream(preference, level_tx.cloned()) {
                    Ok(stream) => {
                        active = Some(stream);
                        Ok(())
                    }
                    Err(error) => Err(error),
                };
                let _ = reply.send(outcome);
            }
            Command::Stop(reply) => {
                let Some(stream) = active.take() else {
                    let _ = reply.send(Err(anyhow!("stop called before start")));
                    continue;
                };
                let _ = reply.send(Ok(finish(stream)));
            }
            Command::Cancel(reply) => {
                // Dropping the stream is the cancel; the samples go with it.
                active = None;
                let _ = reply.send(Ok(()));
            }
        }
    }
    // The channel closed: drop any live stream so the device is released.
    drop(active);
    tracing::debug!("voice: capture thread stopped");
}

/// Stop a stream and hand back its audio at [`TARGET_SAMPLE_RATE`].
fn finish(stream: ActiveStream) -> Vec<f32> {
    let ActiveStream {
        _stream,
        buffer,
        sample_rate,
    } = stream;
    // Drop the stream before reading the buffer so no callback is still
    // appending to it.
    drop(_stream);

    let (samples, truncated) = match buffer.lock() {
        Ok(mut guard) => (std::mem::take(&mut guard.samples), guard.truncated),
        Err(poisoned) => {
            // A panicking audio callback poisons the lock. The samples that
            // did arrive are still valid audio, so use them rather than
            // discarding the recording.
            let mut guard = poisoned.into_inner();
            (std::mem::take(&mut guard.samples), guard.truncated)
        }
    };
    if truncated {
        tracing::warn!("voice: recording hit the {MAX_CAPTURE_SECONDS}s ceiling and was truncated");
    }
    resample_linear(&samples, sample_rate, TARGET_SAMPLE_RATE)
}

fn start_stream(
    preference: &str,
    level_tx: Option<mpsc::Sender<f32>>,
) -> anyhow::Result<ActiveStream> {
    let device = resolve_input_device(preference)?;
    let supported = device
        .default_input_config()
        .context("the audio input device reported no usable capture format")?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0;
    let buffer = Arc::new(Mutex::new(CaptureBuffer::new(sample_rate)));
    let level_every = (sample_rate / LEVEL_UPDATES_PER_SECOND).max(1) as usize;

    let on_error = |error| tracing::warn!("voice: audio stream error: {error}");
    let stream = match sample_format {
        SampleFormat::F32 => build::<f32>(
            &device,
            &config,
            channels,
            &buffer,
            level_tx,
            level_every,
            on_error,
        ),
        SampleFormat::I16 => build::<i16>(
            &device,
            &config,
            channels,
            &buffer,
            level_tx,
            level_every,
            on_error,
        ),
        SampleFormat::U16 => build::<u16>(
            &device,
            &config,
            channels,
            &buffer,
            level_tx,
            level_every,
            on_error,
        ),
        SampleFormat::I8 => build::<i8>(
            &device,
            &config,
            channels,
            &buffer,
            level_tx,
            level_every,
            on_error,
        ),
        SampleFormat::I32 => build::<i32>(
            &device,
            &config,
            channels,
            &buffer,
            level_tx,
            level_every,
            on_error,
        ),
        SampleFormat::F64 => build::<f64>(
            &device,
            &config,
            channels,
            &buffer,
            level_tx,
            level_every,
            on_error,
        ),
        other => {
            return Err(anyhow!(
                "the audio input device uses sample format {other:?}, which archon cannot read"
            ));
        }
    }
    .context("could not open the audio input stream")?;

    stream.play().context("could not start the microphone")?;
    tracing::info!("voice: recording at {sample_rate} Hz, {channels} channel(s)");

    Ok(ActiveStream {
        _stream: stream,
        buffer,
        sample_rate,
    })
}

/// Build a stream for one concrete sample format.
///
/// Every format differs only in how one sample becomes an `f32`, which is what
/// `f32::from_sample` supplies, so the six arms above share this body.
#[allow(clippy::too_many_arguments)]
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: &Arc<Mutex<CaptureBuffer>>,
    level_tx: Option<mpsc::Sender<f32>>,
    level_every: usize,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    let mut since_level = 0usize;
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mono = mix_frames_to_mono(data, channels, <f32 as FromSample<T>>::from_sample_);
            match buffer.lock() {
                Ok(mut guard) => guard.push(&mono),
                // Nothing to do from an audio callback but keep the device
                // running; `finish` recovers the samples from the poisoned lock.
                Err(_) => return,
            }
            let Some(tx) = level_tx.as_ref() else {
                return;
            };
            since_level += mono.len();
            if since_level < level_every {
                return;
            }
            since_level = 0;
            // try_send, never send: a full meter channel must not stall capture.
            let _ = tx.try_send(rms(&mono));
        },
        on_error,
        None,
    )
}

/// Resolve `preference` to a device: a name, or `"default"`/empty.
fn resolve_input_device(preference: &str) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    let preference = preference.trim();
    if preference.is_empty() || preference == "default" {
        return host.default_input_device().ok_or_else(|| {
            anyhow!("no default audio input device; connect a microphone or set voice.device")
        });
    }
    let devices = host.input_devices().with_context(|| {
        format!("could not enumerate audio input devices while looking for {preference:?}")
    })?;
    let mut seen = Vec::new();
    for device in devices {
        match device.name() {
            Ok(name) if name == preference => return Ok(device),
            Ok(name) => seen.push(name),
            Err(_) => {}
        }
    }
    Err(anyhow!(
        "no audio input device named {preference:?}; available: {}",
        if seen.is_empty() {
            "(none)".to_string()
        } else {
            seen.join(", ")
        }
    ))
}

/// The name of the device `preference` selects, for the startup log.
fn describe_input_device(preference: &str) -> anyhow::Result<String> {
    let device = resolve_input_device(preference)?;
    Ok(device.name().unwrap_or_else(|_| "(unnamed)".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_stops_at_the_ceiling_and_says_it_did() {
        let mut buffer = CaptureBuffer::new(4);
        buffer.max_samples = 5;
        buffer.push(&[0.1; 3]);
        assert!(!buffer.truncated);
        buffer.push(&[0.2; 4]);
        assert_eq!(buffer.samples.len(), 5);
        assert!(
            buffer.truncated,
            "a silently shortened recording is worse than a warned one"
        );
    }

    /// The prefix that fits must be kept, or a capture that overruns loses the
    /// whole callback rather than its tail.
    #[test]
    fn an_overrunning_block_keeps_the_part_that_fits() {
        let mut buffer = CaptureBuffer::new(1);
        buffer.max_samples = 3;
        buffer.push(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(buffer.samples, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn pushing_to_a_full_buffer_adds_nothing() {
        let mut buffer = CaptureBuffer::new(1);
        buffer.max_samples = 2;
        buffer.push(&[1.0, 2.0]);
        buffer.push(&[3.0]);
        assert_eq!(buffer.samples, vec![1.0, 2.0]);
        assert!(buffer.truncated);
    }

    #[test]
    fn the_ceiling_is_five_minutes_of_whatever_rate_the_device_uses() {
        assert_eq!(CaptureBuffer::new(48_000).max_samples, 48_000 * 300);
        assert_eq!(CaptureBuffer::new(16_000).max_samples, 16_000 * 300);
    }

    /// Naming a device that does not exist must list what does, or the user is
    /// left guessing at the spelling.
    #[test]
    fn an_unknown_device_name_is_refused_rather_than_falling_back_to_default() {
        // `cpal::Device` has no Debug, so unwrap the error by hand.
        let error = match resolve_input_device("no-such-microphone-9d3f") {
            Ok(_) => panic!("an unknown device name must not silently resolve"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("no-such-microphone-9d3f"),
            "the error must name what was asked for: {error}"
        );
    }
}
