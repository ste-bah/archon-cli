//! TUI-328: integration tests that drive `run_with_backend` through a
//! representative set of `TuiEvent` variants so branches of the extracted
//! `event_loop::run_inner` are exercised by coverage.
//!
//! Rationale: TUI-310 moved the event-loop body from `app.rs` into
//! `event_loop.rs`, but the only test exercising `run_inner` pre-TUI-328
//! was `app_run_e2e` which sends `GenerationStarted`, `TextDelta`,
//! `TurnComplete`, `Done`. This file adds a second test that walks a wider
//! event surface (tool events, model changes, permission prompt, session
//! picker, MCP manager, theme, vim, voice, agent info, error, session
//! rename, resize) so the branches in `run_inner`'s big match arm become
//! line-covered.
//!
//! No assertions target internal state we can't observe from outside the
//! public `run_with_backend` seam; instead we assert the loop (a) exits
//! cleanly within a timeout, (b) rendered *something*, and (c) survived
//! every event without the `Done` path panicking. That is enough for
//! coverage — the correctness of each event handler is covered by unit
//! tests on `App::on_*` methods.

use std::time::Duration;

use archon_tui::app::{AppConfig, McpServerEntry, SessionPickerEntry, TuiEvent, run_with_backend};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc;

fn buffer_nonempty(terminal: &Terminal<TestBackend>) -> bool {
    let buffer = terminal.backend().buffer().clone();
    let area = buffer.area;
    for y in 0..area.height {
        for x in 0..area.width {
            let sym = buffer[(x, y)].symbol();
            if !sym.is_empty() && sym != " " {
                return true;
            }
        }
    }
    false
}

/// Drive `run_with_backend` through a wide event surface and verify the
/// loop exits cleanly on `TuiEvent::Done`. Each `send` below hits a
/// distinct arm of the event dispatch match inside `run_inner`.
///
/// TASK-200: `#[serial]` because this test dispatches `TuiEvent::Resize
/// { cols: 100, rows: 30 }` which writes to the process-global
/// `LAST_KNOWN_SIZE`. It does not read back, but it still races against
/// any in-binary test that does. Coordinated with the default-key
/// `#[serial]` tests elsewhere in the crate's test graph.
#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_with_backend_walks_wide_event_surface() {
    let (event_tx, event_rx) = mpsc::unbounded_channel::<TuiEvent>();
    let (input_tx, _input_rx) = mpsc::channel::<String>(16);

    let config = AppConfig {
        event_rx,
        input_tx,
        splash: None,
        btw_tx: None,
        permission_tx: None,
        command_catalog: Vec::new(),
    };

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("TestBackend");

    // Event scripter — fire many events with small delays so the loop
    // gets multiple draw cycles between them.
    let scripter = tokio::spawn(async move {
        // Session rename + permission mode change
        let _ = event_tx.send(TuiEvent::SessionRenamed("my-session".into()));
        let _ = event_tx.send(TuiEvent::PermissionModeChanged("acceptEdits".into()));
        // Model change
        let _ = event_tx.send(TuiEvent::ModelChanged("claude-sonnet-4-9".into()));
        // Generation + text + thinking
        let _ = event_tx.send(TuiEvent::GenerationStarted);
        let _ = event_tx.send(TuiEvent::ThinkingDelta("pondering...".into()));
        let _ = event_tx.send(TuiEvent::ThinkingToggle(true));
        let _ = event_tx.send(TuiEvent::TextDelta("hello world".into()));
        // Tool lifecycle
        let _ = event_tx.send(TuiEvent::ToolStart {
            name: "Bash".into(),
            id: "tool-1".into(),
        });
        let _ = event_tx.send(TuiEvent::ToolComplete {
            name: "Bash".into(),
            id: "tool-1".into(),
            success: true,
            output: "ok".into(),
        });
        let _ = event_tx.send(TuiEvent::ToolStart {
            name: "Edit".into(),
            id: "tool-2".into(),
        });
        let _ = event_tx.send(TuiEvent::ToolComplete {
            name: "Edit".into(),
            id: "tool-2".into(),
            success: false,
            output: "permission denied".into(),
        });
        // Turn complete — triggers pending_input drain path.
        let _ = event_tx.send(TuiEvent::TurnComplete {
            input_tokens: 50,
            output_tokens: 120,
        });
        // Theme + accent color + vim + voice + agent info
        let _ = event_tx.send(TuiEvent::SetAccentColor(ratatui::style::Color::Cyan));
        let _ = event_tx.send(TuiEvent::SetTheme("intj".into()));
        let _ = event_tx.send(TuiEvent::SetVimMode(true));
        let _ = event_tx.send(TuiEvent::VimToggle); // turns it off
        let _ = event_tx.send(TuiEvent::VimToggle); // turns it on
        let _ = event_tx.send(TuiEvent::VoiceText("typed from voice".into()));
        let _ = event_tx.send(TuiEvent::SetAgentInfo {
            name: "reviewer".into(),
            color: Some("#ff00ff".into()),
        });
        // Permission prompt
        let _ = event_tx.send(TuiEvent::PermissionPrompt {
            tool: "Write".into(),
            description: "writing a file".into(),
        });
        // Session picker + MCP manager ShowMcpManager + UpdateMcpManager
        let _ = event_tx.send(TuiEvent::ShowSessionPicker(vec![SessionPickerEntry {
            id: "sid-1".into(),
            name: "first".into(),
            turns: 3,
            cost: 0.05,
            last_active: "2m".into(),
        }]));
        let _ = event_tx.send(TuiEvent::ShowMcpManager(vec![McpServerEntry {
            name: "srv-a".into(),
            state: "ready".into(),
            tool_count: 1,
            disabled: false,
            tools: vec!["t".into()],
        }]));
        let _ = event_tx.send(TuiEvent::UpdateMcpManager(vec![McpServerEntry {
            name: "srv-a".into(),
            state: "crashed".into(),
            tool_count: 0,
            disabled: false,
            tools: vec![],
        }]));
        // Error + slash-command complete + resize + btw response
        let _ = event_tx.send(TuiEvent::BtwResponse("side note".into()));
        let _ = event_tx.send(TuiEvent::Error("boom".into()));
        let _ = event_tx.send(TuiEvent::SlashCommandComplete);
        let _ = event_tx.send(TuiEvent::Resize {
            cols: 100,
            rows: 30,
        });
        // TUI-106 no-op arms (run_tui path just drops these) — still needs
        // to hit the match arms.
        let _ = event_tx.send(TuiEvent::UserInput("ignored in this loop".into()));
        let _ = event_tx.send(TuiEvent::SlashCancel);
        let _ = event_tx.send(TuiEvent::SlashAgent("reviewer".into()));
        // Let the TUI settle, then terminate.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = event_tx.send(TuiEvent::Done);
    });

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        run_with_backend(config, &mut terminal),
    )
    .await;

    let _ = scripter.await;

    let run_result = result.expect("run_with_backend timed out");
    assert!(
        run_result.is_ok(),
        "run_with_backend returned error: {:?}",
        run_result.err()
    );
    assert!(
        buffer_nonempty(&terminal),
        "loop ran but rendered nothing — run_inner never called draw?"
    );
}
