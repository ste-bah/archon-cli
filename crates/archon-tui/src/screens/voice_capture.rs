//! The voice capture overlay (#192).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! Restored with its rendering rewritten. What came back drew its own borders,
//! ignored the theme, and built a "waveform" by concatenating one run of `=`
//! per sample into a single line it then truncated at 80 characters — which is
//! not a waveform, it is a bar whose length is the sum of the amplitudes.
//!
//! The rest of the screen was sound, and this is the one overlay that shows
//! *live* state rather than a list, so the point of wiring it is that every
//! field has a real source: [`push_sample`](VoiceCaptureOverlay::push_sample)
//! is fed by the capture thread's level meter,
//! [`set_transcription`](VoiceCaptureOverlay::set_transcription) by the STT
//! reply, and the threshold line is the VAD threshold from `config.toml`, so
//! "why did nothing happen" is answerable on screen: the meter never crossed it.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

/// How many level readings the meter remembers.
///
/// At the capture thread's 20 Hz that is ten seconds of history, which is
/// wider than any terminal will draw; the render trims to the region.
const HISTORY: usize = 200;

/// Level glyphs, quietest first. A blank for "no signal at all" so silence
/// reads as silence rather than as a row of low bars.
const BARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Full-scale level for the meter.
///
/// Speech into a normally-gained microphone peaks well below 1.0, and a meter
/// scaled to digital full scale would sit in its bottom tenth and look broken.
const METER_FULL_SCALE: f32 = 0.5;

/// Live state of a voice capture.
#[derive(Debug)]
pub struct VoiceCaptureOverlay {
    /// Rolling RMS levels, oldest first.
    waveform: VecDeque<f32>,
    /// The last transcription, once one has arrived.
    transcription: String,
    /// True between the start of a recording and its end.
    is_recording: bool,
    /// The VAD threshold a recording has to beat to be transcribed at all.
    vad_threshold: f32,
}

impl VoiceCaptureOverlay {
    pub fn new() -> Self {
        Self {
            waveform: VecDeque::with_capacity(HISTORY),
            transcription: String::new(),
            is_recording: false,
            vad_threshold: 0.02,
        }
    }

    /// Build with the VAD threshold that is actually configured.
    ///
    /// Without this the marked line would be a decoration: a screen that draws
    /// a threshold the pipeline does not use tells the user the wrong thing
    /// about why their recording was discarded.
    pub fn with_threshold(vad_threshold: f32) -> Self {
        Self {
            vad_threshold,
            ..Self::new()
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

    /// Get VAD threshold.
    pub fn vad_threshold(&self) -> f32 {
        self.vad_threshold
    }

    /// Start recording: a new recording starts from silence, not from the last
    /// one's tail.
    pub fn start(&mut self) {
        self.is_recording = true;
        self.waveform.clear();
        self.transcription.clear();
    }

    /// Stop recording. The levels stay on screen — they are the evidence for
    /// whatever the transcription turns out to say, or fail to.
    pub fn stop(&mut self) {
        self.is_recording = false;
    }

    /// Clear waveform and transcription.
    pub fn clear(&mut self) {
        self.waveform.clear();
        self.transcription.clear();
        self.is_recording = false;
    }

    /// Append a level reading.
    pub fn push_sample(&mut self, sample: f32) {
        if self.waveform.len() >= HISTORY {
            self.waveform.pop_front();
        }
        self.waveform.push_back(sample);
    }

    /// Set transcription text.
    pub fn set_transcription(&mut self, text: &str) {
        self.transcription = text.to_string();
    }

    /// The loudest level seen in this recording.
    ///
    /// This is what decides whether the VAD will accept the recording, so it is
    /// the number worth putting next to the threshold.
    pub fn peak(&self) -> f32 {
        self.waveform.iter().copied().fold(0.0_f32, f32::max)
    }

    /// Render the overlay.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Voice — Ctrl+V record/stop · Esc cancel · Enter close ";

        // status + blank + meter + scale + blank + transcription + 2 borders.
        let (region, block) = crate::overlay::open(f, area, 9, TITLE, theme);
        let usable = region.width.saturating_sub(4).max(1) as usize;

        let status = if self.is_recording {
            Span::styled(
                "● recording",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else if self.waveform.is_empty() {
            Span::styled("○ idle", Style::default().fg(theme.muted))
        } else {
            Span::styled("○ stopped", Style::default().fg(theme.muted))
        };

        let peak = self.peak();
        let audible = peak > self.vad_threshold;
        let verdict = if self.waveform.is_empty() {
            String::new()
        } else if audible {
            format!(
                "  peak {peak:.3} (above the {:.3} threshold)",
                self.vad_threshold
            )
        } else {
            // The failure the meter exists to explain.
            format!(
                "  peak {peak:.3} — below the {:.3} threshold, so this will be discarded",
                self.vad_threshold
            )
        };

        let transcription = if self.transcription.is_empty() {
            Span::styled(
                "(nothing transcribed yet)",
                Style::default().fg(theme.muted),
            )
        } else {
            Span::styled(
                self.transcription.clone(),
                crate::overlay::body_style(theme),
            )
        };

        let lines = vec![
            Line::from(vec![
                status,
                Span::styled(verdict, Style::default().fg(theme.muted)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                meter(&self.waveform, usable),
                Style::default().fg(theme.accent),
            )),
            Line::from(Span::styled(
                scale_caption(usable),
                Style::default().fg(theme.muted),
            )),
            Line::from(""),
            Line::from(transcription),
        ];

        f.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(ratatui::widgets::Wrap { trim: false }),
            region,
        );
    }
}

/// The most recent `width` levels as a bar graph, oldest left.
///
/// Levels are clamped to [`METER_FULL_SCALE`] rather than to 1.0 so ordinary
/// speech uses the height of the glyph range instead of its bottom step.
fn meter(levels: &VecDeque<f32>, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let skip = levels.len().saturating_sub(width);
    let mut out = String::with_capacity(width * 3);
    for level in levels.iter().skip(skip) {
        let scaled = (level / METER_FULL_SCALE).clamp(0.0, 1.0);
        let step = (scaled * (BARS.len() - 1) as f32).round() as usize;
        out.push_str(BARS[step.min(BARS.len() - 1)]);
    }
    if out.trim().is_empty() && !levels.is_empty() {
        // All-silent history: say so, because a row of blanks is
        // indistinguishable from a meter that is not running.
        return "(silence)".to_string();
    }
    if levels.is_empty() {
        return "(no audio yet)".to_string();
    }
    out
}

/// A caption naming which end of the meter is now.
fn scale_caption(width: usize) -> String {
    const LEFT: &str = "older";
    const RIGHT: &str = "now";
    if width < LEFT.len() + RIGHT.len() + 1 {
        return RIGHT.to_string();
    }
    let gap = width - LEFT.len() - RIGHT.len();
    format!("{LEFT}{}{RIGHT}", " ".repeat(gap))
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
        assert!(overlay.waveform.len() <= HISTORY);
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

    /// Otherwise the second recording is judged against the first one's levels.
    #[test]
    fn starting_a_recording_drops_the_previous_one() {
        let mut overlay = VoiceCaptureOverlay::new();
        overlay.push_sample(0.4);
        overlay.set_transcription("the last thing I said");
        overlay.start();
        assert!(overlay.waveform_slice().is_empty());
        assert!(overlay.transcription().is_empty());
    }

    /// The levels are the evidence for the transcription, so stopping must not
    /// wipe them.
    #[test]
    fn stopping_keeps_the_levels_on_screen() {
        let mut overlay = VoiceCaptureOverlay::new();
        overlay.start();
        overlay.push_sample(0.3);
        overlay.stop();
        assert_eq!(overlay.waveform_slice(), vec![0.3]);
    }

    #[test]
    fn the_peak_is_the_loudest_reading() {
        let mut overlay = VoiceCaptureOverlay::new();
        for level in [0.01, 0.42, 0.09] {
            overlay.push_sample(level);
        }
        assert!((overlay.peak() - 0.42).abs() < 1e-6);
        assert_eq!(VoiceCaptureOverlay::new().peak(), 0.0);
    }

    #[test]
    fn the_configured_threshold_is_the_one_that_is_drawn() {
        assert!((VoiceCaptureOverlay::with_threshold(0.07).vad_threshold() - 0.07).abs() < 1e-6);
    }

    #[test]
    fn the_meter_is_as_wide_as_the_space_it_is_given() {
        let levels: VecDeque<f32> = (0..50).map(|i| i as f32 * 0.01).collect();
        assert_eq!(meter(&levels, 10).chars().count(), 10);
        assert_eq!(meter(&levels, 0), "");
    }

    /// Louder must draw taller, or the meter is decoration.
    #[test]
    fn a_louder_level_draws_a_taller_bar() {
        let quiet: VecDeque<f32> = VecDeque::from(vec![0.05]);
        let loud: VecDeque<f32> = VecDeque::from(vec![0.5]);
        assert_ne!(meter(&quiet, 1), meter(&loud, 1));
        assert_eq!(meter(&loud, 1), "█");
    }

    #[test]
    fn an_all_silent_recording_says_so_rather_than_drawing_blanks() {
        let silent: VecDeque<f32> = VecDeque::from(vec![0.0; 20]);
        assert_eq!(meter(&silent, 20), "(silence)");
        assert_eq!(meter(&VecDeque::new(), 20), "(no audio yet)");
    }

    #[test]
    fn the_caption_survives_a_narrow_overlay() {
        assert_eq!(scale_caption(3), "now");
        assert_eq!(scale_caption(10).chars().count(), 10);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(overlay: &VoiceCaptureOverlay) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
        terminal
            .draw(|frame| overlay.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw voice overlay");
        terminal
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn an_idle_overlay_says_it_is_idle_and_names_the_keys() {
        let rendered = text(&draw(&VoiceCaptureOverlay::new()));
        assert!(rendered.contains("idle"), "{rendered}");
        assert!(rendered.contains("Esc cancel"), "{rendered}");
        assert!(rendered.contains("no audio yet"), "{rendered}");
    }

    #[test]
    fn a_live_recording_is_visibly_recording() {
        let mut overlay = VoiceCaptureOverlay::new();
        overlay.start();
        overlay.push_sample(0.6);
        let rendered = text(&draw(&overlay));
        assert!(rendered.contains("recording"), "{rendered}");
        assert!(
            rendered.contains('█'),
            "a level at full scale drew no bar:\n{rendered}"
        );
    }

    /// The whole reason to show a meter: a recording too quiet for the VAD is
    /// discarded, and the user has to be able to see that it was.
    #[test]
    fn a_recording_below_the_threshold_is_told_it_will_be_discarded() {
        let mut overlay = VoiceCaptureOverlay::with_threshold(0.2);
        overlay.start();
        overlay.push_sample(0.01);
        let rendered = text(&draw(&overlay));
        assert!(rendered.contains("discarded"), "{rendered}");
    }

    #[test]
    fn a_transcription_is_drawn_once_it_arrives() {
        let mut overlay = VoiceCaptureOverlay::new();
        overlay.push_sample(0.3);
        overlay.set_transcription("add the parser");
        let rendered = text(&draw(&overlay));
        assert!(rendered.contains("add the parser"), "{rendered}");
    }

    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&VoiceCaptureOverlay::new());
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(bottom.trim().is_empty(), "painted the status-bar row");
    }
}
