//! Event / snapshot fixtures loaded from tests/fixtures/.
//!
//! Minimal reimplementation pinned to pre-refactor visual output. The real
//! archon-tui::app rendering is not yet public API; these fixtures construct
//! equivalent Buffers via ratatui primitives so phase-3 modularization must
//! converge to them.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

/// Splash screen: centered box with title "Archon" and subtitle "press any key".
pub fn splash_screen_buffer() -> Buffer {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    terminal
        .draw(|f| {
            let area = f.area();
            let block = Block::default().borders(Borders::ALL).title("Archon");
            let lines = vec![
                Line::from(Span::raw("Archon")),
                Line::from(Span::raw("press any key")),
            ];
            let paragraph = Paragraph::new(lines)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(paragraph, area);
        })
        .expect("draw splash");
    terminal.backend().buffer().clone()
}

/// Idle prompt: top pane titled "Archon", bottom pane shows "> {user_text}".
pub fn idle_prompt_buffer(user_text: &str) -> Buffer {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    let prompt_text = format!("> {}", user_text);
    terminal
        .draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
                .split(area);

            let upper = Block::default().borders(Borders::ALL).title("Archon");
            f.render_widget(upper, chunks[0]);

            let lower_block = Block::default().borders(Borders::ALL);
            let lower =
                Paragraph::new(Line::from(Span::raw(prompt_text.clone()))).block(lower_block);
            f.render_widget(lower, chunks[1]);
        })
        .expect("draw idle prompt");
    terminal.backend().buffer().clone()
}

/// Inflight agent: bold header "Agent: {name} ({ms}ms)" then "working..." below.
pub fn inflight_agent_buffer(agent_name: &str, elapsed_ms: u64) -> Buffer {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    let header_text = format!("Agent: {} ({}ms)", agent_name, elapsed_ms);
    terminal
        .draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(0),
                    ]
                    .as_ref(),
                )
                .split(area);

            let header = Paragraph::new(Line::from(Span::styled(
                header_text.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            f.render_widget(header, chunks[0]);

            let status = Paragraph::new(Line::from(Span::raw("working...")));
            f.render_widget(status, chunks[1]);
        })
        .expect("draw inflight agent");
    terminal.backend().buffer().clone()
}

/// Error toast: red-bordered "Error" block centered vertically with wrapped msg.
pub fn error_toast_buffer(msg: &str) -> Buffer {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    let body = msg.to_string();
    terminal
        .draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Min(0),
                        Constraint::Length(5),
                        Constraint::Min(0),
                    ]
                    .as_ref(),
                )
                .split(area);
            let middle: Rect = chunks[1];

            let block = Block::default()
                .borders(Borders::ALL)
                .title("Error")
                .border_style(Style::default().fg(Color::Red));
            let paragraph = Paragraph::new(body.clone())
                .block(block)
                .wrap(Wrap { trim: true });
            f.render_widget(paragraph, middle);
        })
        .expect("draw error toast");
    terminal.backend().buffer().clone()
}

/// Modal overlay: block with borders + custom title, centered body paragraph.
pub fn modal_overlay_buffer(title: &str, body: &str) -> Buffer {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    let title_owned = title.to_string();
    let body_owned = body.to_string();
    terminal
        .draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Min(0),
                        Constraint::Length(10),
                        Constraint::Min(0),
                    ]
                    .as_ref(),
                )
                .split(area);
            let middle: Rect = chunks[1];

            let block = Block::default()
                .borders(Borders::ALL)
                .title(title_owned.clone());
            let paragraph = Paragraph::new(body_owned.clone())
                .block(block)
                .wrap(Wrap { trim: true });
            f.render_widget(paragraph, middle);
        })
        .expect("draw modal overlay");
    terminal.backend().buffer().clone()
}
