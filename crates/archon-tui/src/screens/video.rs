//! Video evidence browser screen.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use super::evidence_browser::{EvidenceBrowser, EvidenceRow};
use crate::theme::Theme;
use crate::virtual_list::VirtualList;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSourceItem {
    pub video_id: String,
    pub title: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSegmentItem {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGroupItem {
    pub start_ms: u64,
    pub end_ms: u64,
    pub image_path: String,
    pub detail: String,
}

#[derive(Debug)]
pub struct VideoScreen {
    sources: EvidenceBrowser<VideoSourceItem>,
    transcript: VirtualList<TranscriptSegmentItem>,
    frames: VirtualList<FrameGroupItem>,
    progress_status: String,
    progress_count: u32,
    latest_text: String,
    summary_text: String,
}

impl VideoScreen {
    pub fn sources() -> Self {
        Self {
            sources: EvidenceBrowser::new(8),
            transcript: VirtualList::new(Vec::new(), 8),
            frames: VirtualList::new(Vec::new(), 8),
            progress_status: "idle".into(),
            progress_count: 0,
            latest_text: String::new(),
            summary_text: "no summary (disabled by policy)".into(),
        }
    }

    pub fn set_source_rows(&mut self, rows: Vec<VideoSourceItem>) {
        self.sources.set_rows(rows);
    }

    pub fn set_transcript_segments(&mut self, rows: Vec<TranscriptSegmentItem>) {
        self.transcript.set_items(rows);
    }

    pub fn set_frame_rows(&mut self, rows: Vec<FrameGroupItem>) {
        self.frames.set_items(rows);
    }

    pub fn set_summary(&mut self, summary: impl Into<String>) {
        let summary = summary.into();
        self.summary_text = if summary.trim().is_empty() {
            "no summary (disabled by policy)".into()
        } else {
            summary
        };
    }

    pub fn on_progress(&mut self, segment_count: u32, latest_text: String, status: String) {
        self.progress_count = segment_count;
        self.latest_text = latest_text;
        self.progress_status = status;
    }

    pub fn selected(&self) -> Option<&VideoSourceItem> {
        self.sources.selected()
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(36),
                Constraint::Percentage(20),
                Constraint::Percentage(24),
                Constraint::Percentage(20),
            ])
            .split(area);

        self.sources
            .render(f, rows[0], theme, "Video Sources".to_string());
        self.render_progress(f, rows[1], theme);
        self.render_lists(f, rows[2], theme);
        self.render_summary(f, rows[3], theme);
    }

    fn render_progress(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let lines = vec![
            Line::from(vec![
                Span::styled("status ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(&self.progress_status),
            ]),
            Line::from(format!("segments: {}", self.progress_count)),
            Line::from(format!("latest: {}", self.latest_text)),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .block(block("Ingest Status", theme))
                .wrap(Wrap { trim: true }),
            area,
        );
    }

    fn render_lists(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        f.render_widget(
            List::new(
                self.transcript
                    .visible_items()
                    .iter()
                    .map(transcript_item)
                    .collect::<Vec<_>>(),
            )
            .block(block("Transcript", theme)),
            cols[0],
        );
        f.render_widget(
            List::new(
                self.frames
                    .visible_items()
                    .iter()
                    .map(frame_item)
                    .collect::<Vec<_>>(),
            )
            .block(block("Frames", theme)),
            cols[1],
        );
    }

    fn render_summary(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        f.render_widget(
            Paragraph::new(self.summary_text.as_str())
                .block(block("Summary", theme))
                .wrap(Wrap { trim: true }),
            area,
        );
    }
}

impl EvidenceRow for VideoSourceItem {
    fn id(&self) -> &str {
        &self.video_id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn status(&self) -> &str {
        &self.status
    }

    fn detail(&self) -> &str {
        &self.detail
    }
}

fn block<'a>(title: &'a str, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(theme.header)
        .border_style(Style::default().fg(theme.border))
}

fn transcript_item(row: &TranscriptSegmentItem) -> ListItem<'_> {
    let speaker = row
        .speaker
        .as_ref()
        .filter(|value| !value.is_empty())
        .map(|value| format!("[{value}] "))
        .unwrap_or_default();
    ListItem::new(format!(
        "{}-{}  {}{}",
        format_ms(row.start_ms),
        format_ms(row.end_ms),
        speaker,
        row.text
    ))
}

fn frame_item(row: &FrameGroupItem) -> ListItem<'_> {
    ListItem::new(format!(
        "{}-{}  {}  {}",
        format_ms(row.start_ms),
        format_ms(row.end_ms),
        row.image_path,
        row.detail
    ))
}

fn format_ms(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_rows_select_first_video() {
        let mut screen = VideoScreen::sources();
        screen.set_source_rows(vec![VideoSourceItem {
            video_id: "video-1".into(),
            title: "Demo".into(),
            status: "success".into(),
            detail: "3 chunks".into(),
        }]);
        assert_eq!(screen.selected().unwrap().video_id, "video-1");
    }

    #[test]
    fn progress_updates_status_count_and_latest_text() {
        let mut screen = VideoScreen::sources();
        screen.on_progress(4, "latest segment".into(), "asr_running".into());
        assert_eq!(screen.progress_count, 4);
        assert_eq!(screen.latest_text, "latest segment");
        assert_eq!(screen.progress_status, "asr_running");
    }
}
