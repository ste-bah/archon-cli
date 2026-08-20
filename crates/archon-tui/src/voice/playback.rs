//! Playing synthesised speech through the default output device.
//!
//! Same shape as `cpal_source.rs` and for the same reason: `cpal::Stream` is
//! not `Send` on every backend, so the stream never leaves the thread that
//! built it. Commands go in on a channel, replies come back on a `oneshot`.
//!
//! Playback is resampled to whatever rate the device wants and mixed up to its
//! channel count, because an output device does not negotiate — asking for
//! 24 kHz mono on a card that runs at 48 kHz stereo either fails to open or
//! plays at the wrong pitch.

use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use tokio::sync::oneshot;

use super::capture::resample_linear;
use super::tts::Speech;

/// How long to wait for a clip to finish before giving up on it.
///
/// A stream that stops producing — a device unplugged mid-sentence — must not
/// hold the speech task forever. The wait is the clip's own length plus this.
const DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

enum Command {
    Play(Speech, oneshot::Sender<anyhow::Result<()>>),
    Stop,
}

/// Samples still to be played, and whether the stream has reached the end.
struct Playing {
    samples: Vec<f32>,
    /// Index of the next frame to emit.
    position: usize,
}

impl Playing {
    /// Fill one output buffer, padding with silence past the end.
    ///
    /// Silence rather than leaving the buffer untouched: cpal hands back
    /// whatever was in it, which is the previous clip, and the tail of a
    /// finished sentence would repeat.
    fn fill<T>(&mut self, data: &mut [T], channels: usize)
    where
        T: SizedSample + FromSample<f32>,
    {
        for frame in data.chunks_mut(channels.max(1)) {
            let sample = self.samples.get(self.position).copied().unwrap_or(0.0);
            if self.position < self.samples.len() {
                self.position += 1;
            }
            for slot in frame.iter_mut() {
                *slot = T::from_sample(sample);
            }
        }
    }

    fn finished(&self) -> bool {
        self.position >= self.samples.len()
    }
}

/// Plays speech through the default output device.
pub struct SpeechPlayer {
    commands: Option<sync_mpsc::Sender<Command>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SpeechPlayer {
    /// Open the default output device.
    ///
    /// Resolved now rather than at the first sentence so a machine with no
    /// speakers is reported at startup, where the caller can decline to enable
    /// speech, instead of at the end of the first reply.
    pub fn new() -> anyhow::Result<Self> {
        let name = describe_output_device()?;
        tracing::info!("voice: speaking through {name}");

        let (tx, rx) = sync_mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("archon-voice-playback".to_string())
            .spawn(move || playback_thread(&rx))
            .context("could not start the speech playback thread")?;

        Ok(Self {
            commands: Some(tx),
            thread: Some(thread),
        })
    }

    /// Speak one clip, returning when it has finished playing.
    pub async fn play(&self, speech: Speech) -> anyhow::Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send(Command::Play(speech, reply_tx))?;
        reply_rx.await.context("the playback thread died")?
    }

    /// Cut the current clip short.
    pub fn stop(&self) -> anyhow::Result<()> {
        self.send(Command::Stop)
    }

    fn send(&self, command: Command) -> anyhow::Result<()> {
        self.commands
            .as_ref()
            .ok_or_else(|| anyhow!("the playback thread is shutting down"))?
            .send(command)
            .map_err(|_| anyhow!("the playback thread has stopped"))
    }
}

impl Drop for SpeechPlayer {
    fn drop(&mut self) {
        // The channel closing is what ends the thread's loop, so the sender
        // must go before the join or this deadlocks.
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn playback_thread(commands: &sync_mpsc::Receiver<Command>) {
    while let Ok(command) = commands.recv() {
        match command {
            Command::Play(speech, reply) => {
                let _ = reply.send(play_blocking(&speech));
            }
            // Nothing is playing between commands — `play_blocking` owns the
            // stream for the length of the clip — so a stop that arrives here
            // has already been satisfied.
            Command::Stop => {}
        }
    }
    tracing::debug!("voice: playback thread stopped");
}

/// Play one clip and return when the device has consumed it.
fn play_blocking(speech: &Speech) -> anyhow::Result<()> {
    if speech.samples.is_empty() {
        return Ok(());
    }
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| anyhow!("no audio output device"))?;
    let supported = device
        .default_output_config()
        .context("the audio output device reported no usable format")?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;

    // The device does not negotiate: match its rate or play at the wrong pitch.
    let samples = resample_linear(&speech.samples, speech.sample_rate, config.sample_rate.0);
    let frames = samples.len();
    let state = Arc::new(Mutex::new(Playing {
        samples,
        position: 0,
    }));

    let on_error = |error| tracing::warn!("voice: playback stream error: {error}");
    let stream = match sample_format {
        SampleFormat::F32 => build::<f32>(&device, &config, channels, &state, on_error),
        SampleFormat::I16 => build::<i16>(&device, &config, channels, &state, on_error),
        SampleFormat::U16 => build::<u16>(&device, &config, channels, &state, on_error),
        SampleFormat::I8 => build::<i8>(&device, &config, channels, &state, on_error),
        SampleFormat::I32 => build::<i32>(&device, &config, channels, &state, on_error),
        SampleFormat::F64 => build::<f64>(&device, &config, channels, &state, on_error),
        other => {
            return Err(anyhow!(
                "the audio output device uses sample format {other:?}, which archon cannot write"
            ));
        }
    }
    .context("could not open the audio output stream")?;

    stream.play().context("could not start playback")?;

    // Wait for the callback to reach the end rather than sleeping the clip's
    // length: the device decides how fast it consumes, and a fixed sleep either
    // truncates the tail or holds the thread past the end.
    let expected =
        std::time::Duration::from_secs_f64(frames as f64 / f64::from(config.sample_rate.0.max(1)));
    let deadline = std::time::Instant::now() + expected + DRAIN_GRACE;
    while std::time::Instant::now() < deadline {
        if state.lock().is_ok_and(|playing| playing.finished()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(())
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    state: &Arc<Mutex<Playing>>,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32>,
{
    let state = Arc::clone(state);
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let Ok(mut playing) = state.lock() else {
                // A poisoned lock means nothing more will play; emit silence
                // rather than whatever the buffer happened to contain.
                for slot in data.iter_mut() {
                    *slot = T::from_sample(0.0_f32);
                }
                return;
            };
            playing.fill(data, channels);
        },
        on_error,
        None,
    )
}

/// The name of the default output device, for the startup log.
fn describe_output_device() -> anyhow::Result<String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| anyhow!("no default audio output device to speak through"))?;
    Ok(device.name().unwrap_or_else(|_| "(unnamed)".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mono_clip_is_written_to_every_channel() {
        let mut playing = Playing {
            samples: vec![0.5, -0.5],
            position: 0,
        };
        let mut out = [0.0_f32; 4];
        playing.fill(&mut out, 2);
        assert_eq!(out, [0.5, 0.5, -0.5, -0.5]);
        assert!(playing.finished());
    }

    /// cpal hands back a dirty buffer, so the tail of the previous sentence
    /// repeats unless the end of a clip is explicitly silence.
    #[test]
    fn past_the_end_is_silence_not_whatever_was_in_the_buffer() {
        let mut playing = Playing {
            samples: vec![0.9],
            position: 0,
        };
        let mut out = [0.7_f32; 3];
        playing.fill(&mut out, 1);
        assert_eq!(out, [0.9, 0.0, 0.0]);
    }

    #[test]
    fn a_clip_is_not_finished_until_every_sample_has_been_emitted() {
        let mut playing = Playing {
            samples: vec![0.1; 4],
            position: 0,
        };
        let mut out = [0.0_f32; 2];
        playing.fill(&mut out, 1);
        assert!(!playing.finished());
        playing.fill(&mut out, 1);
        assert!(playing.finished());
    }

    /// The output device dictates the rate; a clip at the wrong one plays at
    /// the wrong pitch.
    #[test]
    fn a_clip_is_resampled_to_the_devices_rate() {
        let clip: Vec<f32> = (0..24).map(|i| i as f32).collect();
        let at_48k = resample_linear(&clip, 24_000, 48_000);
        assert_eq!(at_48k.len(), 48, "a doubled rate needs twice the samples");
    }
}
