//! Voice capture overlay screen.
//! Layer 1 module — no imports from screens/ or app/.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::Theme;

/// Voice capture overlay state.
#[derive(Debug)]
pub struct VoiceCaptureOverlay {
    /// Rolling waveform buffer (amplitude values).
    waveform: VecDeque<f32>,
    /// Current transcription text.
    transcription: String,
    /// True when recording is active.
    is_recording: bool,
    /// Voice activity detection threshold.
    vad_threshold: f32,
}

impl VoiceCaptureOverlay {
    pub fn new() -> Self {
        Self {
            waveform: VecDeque::with_capacity(200),
            transcription: String::new(),
            is_recording: false,
            vad_threshold: 0.05,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    pub fn transcription(&self) -> &str {
        &self.transcription
    }

    pub fn waveform_slice(&self) -> Vec<f32> {
        self.waveform.iter().copied().collect()
    }

    /// Start recording.
    pub fn start(&mut self) {
        self.is_recording = true;
    }

    /// Stop recording.
    pub fn stop(&mut self) {
        self.is_recording = false;
    }

    /// Clear waveform and transcription.
    pub fn clear(&mut self) {
        self.waveform.clear();
        self.transcription.clear();
        self.is_recording = false;
    }

    /// Append a waveform sample.
    pub fn push_sample(&mut self, sample: f32) {
        if self.waveform.len() >= 200 {
            self.waveform.pop_front();
        }
        self.waveform.push_back(sample);
    }

    /// Set transcription text.
    pub fn set_transcription(&mut self, text: &str) {
        self.transcription = text.to_string();
    }

    /// Get VAD threshold.
    pub fn vad_threshold(&self) -> f32 {
        self.vad_threshold
    }

    /// Render voice capture overlay into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Voice Capture{}{}",
                if self.is_recording { " [recording]" } else { "" },
                if self.transcription.is_empty() { "" } else { " — " },
            ));

        let waveform_str: String = if self.waveform.is_empty() {
            "(no audio)".to_string()
        } else {
            let mut s = String::new();
            for val in &self.waveform {
                let bar = ((val.abs() * 10.0) as usize).min(10);
                s.push_str(&"=".repeat(bar));
            }
            if s.len() > 80 { s.truncate(80); }
            s
        };

        let status = if self.is_recording { "Recording..." } else { "Stopped" };

        let text = format!(
            "{}\n\nTranscription:\n{}\n\nWaveform:\n{}",
            status,
            if self.transcription.is_empty() { "(none)" } else { &self.transcription },
            if waveform_str.is_empty() { "(no signal)" } else { &waveform_str }
        );

        let para = Paragraph::new(text).block(block);
        f.render_widget(para, area);
    }
}

impl Default for VoiceCaptureOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_overlay_not_recording() {
        let overlay = VoiceCaptureOverlay::new();
        assert!(!overlay.is_recording());
        assert!(overlay.transcription().is_empty());
    }

    #[test]
    fn start_stop_recording() {
        let mut overlay = VoiceCaptureOverlay::new();
        overlay.start();
        assert!(overlay.is_recording());
        overlay.stop();
        assert!(!overlay.is_recording());
    }

    #[test]
    fn push_sample_truncates() {
        let mut overlay = VoiceCaptureOverlay::new();
        for i in 0..250 {
            overlay.push_sample((i as f32) * 0.01);
        }
        assert!(overlay.waveform.len() <= 200);
    }

    #[test]
    fn clear_resets() {
        let mut overlay = VoiceCaptureOverlay::new();
        overlay.start();
        overlay.set_transcription("hello");
        overlay.clear();
        assert!(!overlay.is_recording());
        assert!(overlay.transcription().is_empty());
        assert!(overlay.waveform.is_empty());
    }
}