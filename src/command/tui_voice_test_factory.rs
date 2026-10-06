//! Microphone-only substitution for binary unit tests. No hardware fallback.
use std::sync::{Arc, OnceLock};

use archon_tui::voice::pipeline::AudioSource;
use tokio::sync::mpsc;

type AudioFactory = Box<
    dyn Fn(&str, Option<mpsc::Sender<f32>>) -> anyhow::Result<Arc<dyn AudioSource>> + Send + Sync,
>;
static FACTORY: OnceLock<AudioFactory> = OnceLock::new();

pub(crate) fn install(factory: AudioFactory) {
    assert!(
        FACTORY.set(factory).is_ok(),
        "audio factory already installed"
    );
}

pub(crate) fn open_audio(
    device: &str,
    levels: Option<mpsc::Sender<f32>>,
) -> anyhow::Result<Arc<dyn AudioSource>> {
    FACTORY
        .get()
        .expect("voice setup in a unit test requires a controlled audio factory")(device, levels)
}
