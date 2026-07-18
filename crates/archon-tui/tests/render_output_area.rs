//! Output-area rendering regressions extracted from `render_coverage.rs`.

use archon_tui::agent_activity::AgentActivityRow;
use archon_tui::app::{AgentActivityRole, App};
use archon_tui::events::AgentActivityStatus;
use archon_tui::render;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn render_at_width(app: &mut App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).expect("TestBackend");
    terminal
        .draw(|frame| render::draw(frame, app))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let area = buffer.area;
    let mut rendered = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        for x in 0..area.width {
            rendered.push_str(buffer[(x, y)].symbol());
        }
        rendered.push('\n');
    }
    rendered
}

fn render_once(app: &mut App) -> String {
    render_at_width(app, 100)
}

#[test]
fn splash_renders_when_show_splash_true() {
    let mut app = App::new();
    app.show_splash = true;
    app.splash_model = "test-model".into();
    app.splash_working_dir = "/tmp/work".into();
    let rendered = render_once(&mut app);
    assert!(rendered.contains("test-model"), "buffer:\n{rendered}");
}

#[test]
fn output_buffer_renders_when_splash_off() {
    let mut app = App::new();
    app.show_splash = false;
    app.output.append_line("hello world line one");
    app.output.append_line("second line here");
    let rendered = render_once(&mut app);
    assert!(rendered.contains("hello world"), "buffer:\n{rendered}");
    assert!(rendered.contains("second line"), "buffer:\n{rendered}");
}

#[test]
fn output_scrollbar_renders_when_content_overflows() {
    let mut app = App::new();
    app.show_splash = false;
    for i in 0..100 {
        app.output.append_line(&format!("line-{i:03}"));
    }
    let rendered = render_once(&mut app);
    app.output.scroll_to_bottom();
    let rendered_at_bottom = render_once(&mut app);
    assert!(!rendered.is_empty());
    assert!(!rendered_at_bottom.is_empty());
}

#[test]
fn scroll_lock_footer_counts_new_wrapped_rows_until_follow_resumes() {
    let mut app = App::new();
    app.show_splash = false;
    for index in 0..40 {
        app.output.append_line(&format!("existing-{index:02}"));
    }
    let initial = render_once(&mut app);
    assert!(!initial.contains("new lines"));

    app.output.scroll_up(10);
    for index in 0..42 {
        app.output.append_line(&format!("new-{index:02}"));
    }
    let locked = render_once(&mut app);
    assert!(
        locked.contains("▼ 42 new lines — PageDown/End to follow"),
        "buffer:\n{locked}"
    );

    app.output.scroll_to_bottom();
    let following = render_once(&mut app);
    assert!(!following.contains("new lines"), "buffer:\n{following}");
}

#[test]
fn transcript_wrap_geometry_matches_reserved_scrollbar_column() {
    let mut app = App::new();
    app.show_splash = false;
    app.output.append_line("1234567890");

    let rendered = render_at_width(&mut app, 10);
    let mut lines = rendered.lines();

    assert_eq!(lines.next(), Some("123456789 "));
    assert_eq!(lines.next(), Some("0         "));
}

#[test]
fn scroll_lock_footer_does_not_replace_last_transcript_row() {
    let mut app = App::new();
    app.show_splash = false;
    for index in 0..30 {
        app.output.append_line(&format!("existing-{index:02}"));
    }
    app.output.scroll_up(10);
    let before = render_once(&mut app);
    assert!(
        before
            .lines()
            .nth(22)
            .is_some_and(|line| line.trim().is_empty()),
        "locked footer row must already be reserved; buffer:\n{before}"
    );

    app.output.append_line("new-arrival");
    let rendered = render_once(&mut app);

    assert!(rendered.contains("existing-21"), "buffer:\n{rendered}");
    assert!(
        rendered.contains("▼ 1 new lines — PageDown/End to follow"),
        "buffer:\n{rendered}"
    );
}

#[test]
fn output_with_thinking_active_renders_dots() {
    let mut app = App::new();
    app.show_splash = false;
    app.show_thinking = true;
    app.output.append_line("pre-thinking");
    app.thinking.active = true;
    app.thinking.start = Some(std::time::Instant::now());
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Thinking"), "buffer:\n{rendered}");
}

#[test]
fn expanded_thinking_uses_capped_scrollable_region() {
    let mut app = App::new();
    app.show_splash = false;
    app.show_thinking = true;
    app.output.append_line("transcript remains visible");
    app.thinking.active = true;
    app.thinking.expanded = true;
    app.thinking.accumulated = (0..30)
        .map(|index| format!("thought-{index:02}"))
        .collect::<Vec<_>>()
        .join("\n");

    let following = render_once(&mut app);
    assert!(following.contains("transcript remains visible"));
    assert!(following.contains("thought-29"));
    assert!(!following.contains("thought-00"));

    app.thinking.scroll_to_top();
    let scrolled = render_once(&mut app);
    assert!(scrolled.contains("transcript remains visible"));
    assert!(scrolled.contains("thought-00"));
    assert!(!scrolled.contains("thought-29"));
}

#[test]
fn expanded_thinking_follows_tail_beyond_u16_rows() {
    let mut app = App::new();
    app.show_splash = false;
    app.show_thinking = true;
    app.output.append_line("transcript remains visible");
    app.thinking.active = true;
    app.thinking.expanded = true;
    app.thinking.accumulated = (0..70_000)
        .map(|index| format!("thought-{index:05}"))
        .collect::<Vec<_>>()
        .join("\n");

    let following = render_once(&mut app);
    assert!(following.contains("transcript remains visible"));
    assert!(following.contains("thought-69999"));
    assert!(!following.contains("thought-00000"));

    app.thinking.scroll_to_top();
    let top = render_once(&mut app);
    assert!(top.contains("thought-00000"));
    assert!(!top.contains("thought-69999"));

    app.thinking.scroll_to_bottom();
    let bottom = render_once(&mut app);
    assert!(bottom.contains("thought-69999"));
}

#[test]
fn agent_activity_panel_renders_parent_and_subagent_rows() {
    let mut app = App::new();
    app.show_splash = false;
    app.output.append_line("assistant response in progress");
    app.agent_activity = vec![
        AgentActivityRow::new(
            "parent",
            "Parent",
            AgentActivityRole::Parent,
            AgentActivityStatus::WaitingForTool,
        ),
        AgentActivityRow::new(
            "agent-1",
            "Subagent 1",
            AgentActivityRole::Subagent,
            AgentActivityStatus::Running,
        ),
    ];
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Agent Activity"), "buffer:\n{rendered}");
    assert!(rendered.contains("[PARENT]"), "buffer:\n{rendered}");
    assert!(rendered.contains("[AGENT]"), "buffer:\n{rendered}");
}

#[test]
fn agent_activity_panel_renders_canonical_states_and_artifacts() {
    let mut app = App::new();
    app.show_splash = false;
    app.output.append_line("activity source of truth");
    app.agent_activity = vec![
        AgentActivityRow::new(
            "queued",
            "queued-agent",
            AgentActivityRole::Subagent,
            AgentActivityStatus::Queued,
        ),
        AgentActivityRow::new(
            "running",
            "running-agent",
            AgentActivityRole::Subagent,
            AgentActivityStatus::Running,
        ),
        AgentActivityRow::new(
            "waiting",
            "waiting-agent",
            AgentActivityRole::Subagent,
            AgentActivityStatus::Waiting,
        ),
        AgentActivityRow::new(
            "backgrounded",
            "backgrounded",
            AgentActivityRole::Background,
            AgentActivityStatus::Backgrounded,
        ),
        AgentActivityRow::new(
            "failed",
            "failed-agent",
            AgentActivityRole::Subagent,
            AgentActivityStatus::Failed,
        ),
        AgentActivityRow::new(
            "completed",
            "done-agent",
            AgentActivityRole::Subagent,
            AgentActivityStatus::Complete,
        ),
    ];
    app.agent_activity[5].artifact_id = Some("artifact-report-1".into());

    let rendered = render_once(&mut app);
    for expected in [
        "queued",
        "running",
        "waiting",
        "[BG]",
        "failed",
        "done",
        "artifact-report-1",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}; buffer:\n{rendered}"
        );
    }
}
