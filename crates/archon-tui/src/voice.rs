pub mod capture;
pub mod hotkey;
pub mod pipeline;
pub mod stt;

use archon_core::config::VoiceConfig;

/// Current state of the voice input pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceState {
    Idle,
    Listening,
    Transcribing,
    Error(String),
}

/// Manages voice input state and buffered injected text.
pub struct VoiceManager {
    config: VoiceConfig,
    state: VoiceState,
    /// Buffer of injected text segments (tests can inspect this).
    injected: Vec<String>,
}

impl VoiceManager {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            state: VoiceState::Idle,
            injected: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn state(&self) -> &VoiceState {
        &self.state
    }

    pub fn set_state(&mut self, state: VoiceState) {
        self.state = state;
    }

    /// Append `text` to the injected buffer. Returns Ok(()) always.
    pub fn inject_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.injected.push(text.to_owned());
        Ok(())
    }

    /// Drain all buffered injected text segments.
    pub fn drain_injected(&mut self) -> Vec<String> {
        std::mem::take(&mut self.injected)
    }
}
