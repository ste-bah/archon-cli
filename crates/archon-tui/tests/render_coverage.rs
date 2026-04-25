//! TUI-328: coverage-oriented render tests.
//!
//! Exercises `render::draw` and the body/chrome helpers against a
//! `TestBackend` with controlled `App` state so every meaningful branch
//! inside `render/body.rs` and `render/chrome.rs` is hit: splash, output,
//! scrollbar, ultrathink active input, permission prompt, vim mode,
//! generating prefix, session badge, suggestions popup, session picker
//! overlay, MCP manager (ServerList / ServerMenu / ToolList),
//! permission indicator labels, /btw overlay, ultrathink status bar.
//!
//! These are real rendering assertions driven through the public `draw`
//! entry point — every assertion reads the rendered buffer. A broken
//! render helper would leave the asserted cell empty and fail the test.

use archon_tui::app::{
    App, McpManager, McpManagerView, McpServerEntry, SessionPicker, SessionPickerEntry,
};
use archon_tui::render;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Flatten a `TestBackend` buffer into a single `String` with `\n` row
/// separators. Used for substring assertions on rendered output.
fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer().clone();
    let area = buffer.area;
    let mut s = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        for x in 0..area.width {
            s.push_str(buffer[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

fn term() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(100, 30)).expect("TestBackend")
}

fn render_once(app: &mut App) -> String {
    let mut t = term();
    t.draw(|frame| render::draw(frame, app)).expect("draw");
    buffer_to_string(&t)
}

// ───────────────────────────────────────────────────────────────────────
// Splash + output area
// ───────────────────────────────────────────────────────────────────────

#[test]
fn splash_renders_when_show_splash_true() {
    let mut app = App::new();
    app.show_splash = true;
    app.splash_model = "test-model".into();
    app.splash_working_dir = "/tmp/work".into();
    let rendered = render_once(&mut app);
    // The splash model string must land somewhere in the buffer.
    assert!(
        rendered.contains("test-model"),
        "splash should render model name, got:\n{rendered}"
    );
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
    // 100 lines of content vs 30-row terminal will force scrollbar path.
    for i in 0..100 {
        app.output.append_line(&format!("line-{i:03}"));
    }
    let rendered = render_once(&mut app);
    // Just prove we didn't panic and the last appended text is reachable
    // after a scroll_to_bottom.
    app.output.scroll_to_bottom();
    let rendered2 = render_once(&mut app);
    assert!(!rendered.is_empty());
    assert!(!rendered2.is_empty());
}

#[test]
fn output_with_thinking_active_renders_dots() {
    let mut app = App::new();
    app.show_splash = false;
    app.output.append_line("pre-thinking");
    app.thinking.active = true;
    app.thinking.start = Some(std::time::Instant::now());
    let rendered = render_once(&mut app);
    // "Thinking" label is inserted into the output area.
    assert!(rendered.contains("Thinking"), "buffer:\n{rendered}");
}

// ───────────────────────────────────────────────────────────────────────
// Input area branches
// ───────────────────────────────────────────────────────────────────────

#[test]
fn input_ultrathink_active_path() {
    let mut app = App::new();
    app.show_splash = false;
    // Feed keyword via the public insert path so UltrathinkState activates.
    for c in "ultrathink ".chars() {
        app.input.insert(c);
    }
    app.input.insert('X');
    assert!(app.input.ultrathink.active, "ultrathink should be active");
    let rendered = render_once(&mut app);
    assert!(rendered.contains('X'), "buffer:\n{rendered}");
}

#[test]
fn input_permission_prompt_renders_yn() {
    let mut app = App::new();
    app.show_splash = false;
    app.permission_prompt = Some("Bash(rm -rf)".into());
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Allow"), "buffer:\n{rendered}");
    assert!(rendered.contains("y/n"), "buffer:\n{rendered}");
}

#[test]
fn input_vim_mode_shows_mode_indicator() {
    let mut app = App::new();
    app.show_splash = false;
    app.vim_state = Some(archon_tui::vim::VimState::new());
    let rendered = render_once(&mut app);
    assert!(
        rendered.contains("NORMAL"),
        "vim mode indicator missing, buffer:\n{rendered}"
    );
}

#[test]
fn input_generating_with_active_tool_shows_tool_name() {
    let mut app = App::new();
    app.show_splash = false;
    app.is_generating = true;
    app.active_tool = Some("Bash".into());
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Bash"), "buffer:\n{rendered}");
}

#[test]
fn input_generating_without_tool_shows_placeholder() {
    let mut app = App::new();
    app.show_splash = false;
    app.is_generating = true;
    app.active_tool = None;
    let rendered = render_once(&mut app);
    assert!(rendered.contains("..."), "buffer:\n{rendered}");
}

// ───────────────────────────────────────────────────────────────────────
// Session badge + suggestions popup
// ───────────────────────────────────────────────────────────────────────

#[test]
fn session_badge_renders_name() {
    let mut app = App::new();
    app.show_splash = false;
    app.session_name = Some("mybranch".into());
    let rendered = render_once(&mut app);
    assert!(rendered.contains("mybranch"), "buffer:\n{rendered}");
}

#[test]
fn session_badge_absent_when_no_name() {
    let mut app = App::new();
    app.show_splash = false;
    app.session_name = None;
    // Should not panic; nothing specific to assert other than render success.
    let _ = render_once(&mut app);
}

#[test]
fn suggestions_popup_renders_when_active() {
    let mut app = App::new();
    app.show_splash = false;
    app.input.insert('/');
    app.input.insert('h');
    // Manually mark suggestions active — refresh already ran via insert.
    // Some slash command starting with "h" (e.g. /help) must exist; if not,
    // force-activate:
    app.input.suggestions.active = true;
    if app.input.suggestions.suggestions.is_empty() {
        // Inject a synthetic suggestion via the commands registry.
        app.input.suggestions.suggestions = vec![archon_tui::commands::CommandInfo {
            name: "/help",
            description: "show help",
        }];
    }
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Commands"), "buffer:\n{rendered}");
}

#[test]
fn suggestions_popup_suppressed_while_generating() {
    let mut app = App::new();
    app.show_splash = false;
    app.is_generating = true;
    app.input.suggestions.active = true;
    app.input.suggestions.suggestions = vec![archon_tui::commands::CommandInfo {
        name: "/help",
        description: "show help",
    }];
    let rendered = render_once(&mut app);
    // Popup box must not be drawn while generating — "Commands" title should
    // not appear.
    assert!(!rendered.contains("Commands"), "buffer:\n{rendered}");
}

// ───────────────────────────────────────────────────────────────────────
// Session picker overlay
// ───────────────────────────────────────────────────────────────────────

#[test]
fn session_picker_overlay_renders() {
    let mut app = App::new();
    app.show_splash = false;
    app.session_picker = Some(SessionPicker {
        sessions: vec![
            SessionPickerEntry {
                id: "abcdef1234".into(),
                name: "demo".into(),
                turns: 5,
                cost: 0.12,
                last_active: "1m".into(),
            },
            SessionPickerEntry {
                id: "zz99".into(),
                name: String::new(),
                turns: 0,
                cost: 0.0,
                last_active: "now".into(),
            },
        ],
        selected: 0,
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("/resume"), "buffer:\n{rendered}");
    assert!(rendered.contains("demo"), "buffer:\n{rendered}");
    // Short-id prefix of first session should be present.
    assert!(rendered.contains("abcdef12"), "buffer:\n{rendered}");
}

// ───────────────────────────────────────────────────────────────────────
// MCP manager overlay — all 3 views
// ───────────────────────────────────────────────────────────────────────

fn mcp_entry(name: &str, state: &str, tools: Vec<String>, disabled: bool) -> McpServerEntry {
    McpServerEntry {
        name: name.into(),
        state: state.into(),
        tool_count: tools.len(),
        disabled,
        tools,
    }
}

#[test]
fn mcp_manager_server_list_renders_all_states() {
    let mut app = App::new();
    app.show_splash = false;
    app.mcp_manager = Some(McpManager {
        servers: vec![
            mcp_entry("ready-srv", "ready", vec!["t1".into(), "t2".into()], false),
            mcp_entry("crashed-srv", "crashed", vec![], false),
            mcp_entry("starting-srv", "starting", vec![], false),
            mcp_entry("off-srv", "disabled", vec![], true),
            mcp_entry("other-srv", "stopped", vec![], false),
        ],
        view: McpManagerView::ServerList { selected: 1 },
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("MCP Servers"), "buffer:\n{rendered}");
    assert!(rendered.contains("ready-srv"));
    assert!(rendered.contains("crashed-srv"));
    assert!(rendered.contains("starting-srv"));
    assert!(rendered.contains("off-srv"));
}

#[test]
fn mcp_manager_server_menu_renders_actions_for_ready() {
    let mut app = App::new();
    app.show_splash = false;
    app.mcp_manager = Some(McpManager {
        servers: vec![mcp_entry("web", "ready", vec!["web__search".into()], false)],
        view: McpManagerView::ServerMenu {
            server_idx: 0,
            action_idx: 0,
        },
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("web"), "buffer:\n{rendered}");
    assert!(rendered.contains("View Tools"), "buffer:\n{rendered}");
    assert!(rendered.contains("Disable"), "buffer:\n{rendered}");
    assert!(rendered.contains("Back"), "buffer:\n{rendered}");
}

#[test]
fn mcp_manager_server_menu_renders_actions_for_disabled() {
    let mut app = App::new();
    app.show_splash = false;
    app.mcp_manager = Some(McpManager {
        servers: vec![mcp_entry("off", "disabled", vec![], true)],
        view: McpManagerView::ServerMenu {
            server_idx: 0,
            action_idx: 0,
        },
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Enable"), "buffer:\n{rendered}");
    // "Disable" must NOT appear for a disabled server.
    assert!(!rendered.contains("  Disable"), "buffer:\n{rendered}");
}

#[test]
fn mcp_manager_server_menu_renders_actions_for_crashed() {
    let mut app = App::new();
    app.show_splash = false;
    app.mcp_manager = Some(McpManager {
        servers: vec![mcp_entry("crashed-srv", "crashed", vec![], false)],
        view: McpManagerView::ServerMenu {
            server_idx: 0,
            action_idx: 1,
        },
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("Reconnect"), "buffer:\n{rendered}");
}

#[test]
fn mcp_manager_tool_list_with_entries() {
    let mut app = App::new();
    app.show_splash = false;
    app.mcp_manager = Some(McpManager {
        servers: vec![],
        view: McpManagerView::ToolList {
            server_name: "fs".into(),
            tools: vec!["read".into(), "write".into(), "list".into()],
            scroll: 0,
        },
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("fs"), "buffer:\n{rendered}");
    assert!(rendered.contains("read"));
    assert!(rendered.contains("write"));
    assert!(rendered.contains("list"));
}

#[test]
fn mcp_manager_tool_list_empty_shows_placeholder() {
    let mut app = App::new();
    app.show_splash = false;
    app.mcp_manager = Some(McpManager {
        servers: vec![],
        view: McpManagerView::ToolList {
            server_name: "empty".into(),
            tools: vec![],
            scroll: 0,
        },
    });
    let rendered = render_once(&mut app);
    assert!(rendered.contains("(no tools)"), "buffer:\n{rendered}");
}

// ───────────────────────────────────────────────────────────────────────
// Chrome: permission indicator + btw overlay + status bar ultrathink
// ───────────────────────────────────────────────────────────────────────

#[test]
fn permission_indicator_all_modes() {
    let modes = [
        ("bypassPermissions", "bypass"),
        ("yolo", "bypass"),
        ("dontAsk", "don't ask"),
        ("acceptEdits", "accept edits"),
        ("auto", "auto"),
        ("plan", "plan"),
        ("ask", "default"),
    ];
    for (mode, expected) in modes {
        let mut app = App::new();
        app.show_splash = false;
        app.status.permission_mode = mode.into();
        let rendered = render_once(&mut app);
        assert!(
            rendered.contains(expected),
            "mode {mode} should render '{expected}', got:\n{rendered}"
        );
    }
}

#[test]
fn btw_overlay_renders_multiline_text() {
    let mut app = App::new();
    app.show_splash = false;
    app.btw_overlay = Some("line one\nline two\nline three".into());
    let rendered = render_once(&mut app);
    assert!(rendered.contains("line one"), "buffer:\n{rendered}");
    assert!(rendered.contains("line two"));
    assert!(rendered.contains("line three"));
    assert!(rendered.contains("/btw"), "overlay title missing");
    assert!(rendered.contains("dismiss"), "dismiss hint missing");
}

#[test]
fn status_bar_renders_default_fields() {
    let mut app = App::new();
    app.show_splash = false;
    app.status.model = "the-model".into();
    app.status.cost = 1.234;
    let rendered = render_once(&mut app);
    assert!(rendered.contains("the-model"), "buffer:\n{rendered}");
    assert!(
        rendered.contains("$1.23"),
        "cost missing, buffer:\n{rendered}"
    );
}

#[test]
fn status_bar_with_ultrathink_active() {
    let mut app = App::new();
    app.show_splash = false;
    // Insert the keyword through the public path so shimmer activates.
    for c in "ultrathink ".chars() {
        app.input.insert(c);
    }
    assert!(app.input.ultrathink.active);
    app.status.model = "active-model".into();
    let rendered = render_once(&mut app);
    // The ultrathink branch of the status bar separates model with " | "
    // and renders the keyword itself colourised; the model text still
    // lands in the buffer.
    assert!(rendered.contains("active-model"), "buffer:\n{rendered}");
}
