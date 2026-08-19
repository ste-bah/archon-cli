pub mod capture;
#[cfg(feature = "audio-capture")]
pub mod cpal_source;
pub mod pipeline;
pub mod stt;

use std::sync::Arc;

use tokio::sync::mpsc;

/// Open the microphone `device` (`"default"` or a device name).
///
/// This exists so callers need no `cfg` of their own: a binary built without
/// the `audio-capture` feature gets an error saying exactly that, and one built
/// with it gets a real recorder or the reason there isn't one. What no caller
/// gets is a mock — voice input that records silence and reports success is
/// indistinguishable, from the outside, from a microphone nobody is speaking
/// into, which is how the previous wiring hid a subsystem that never worked.
///
/// `level_tx` receives an RMS level while recording, for the capture overlay.
#[cfg_attr(not(feature = "audio-capture"), allow(unused_variables))]
pub fn real_audio_source(
    device: &str,
    level_tx: Option<mpsc::Sender<f32>>,
) -> anyhow::Result<Arc<dyn pipeline::AudioSource>> {
    #[cfg(feature = "audio-capture")]
    {
        Ok(Arc::new(cpal_source::CpalAudioSource::new(
            device, level_tx,
        )?))
    }
    #[cfg(not(feature = "audio-capture"))]
    {
        Err(anyhow::anyhow!(
            "this archon build has no microphone support: rebuild with \
             `--features audio-capture` (Linux also needs libasound2-dev)"
        ))
    }
}
