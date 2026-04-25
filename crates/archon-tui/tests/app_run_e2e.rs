//! End-to-end integration test for `archon_tui::app::run_with_backend`
//! (TUI-327).
//!
//! This test exercises the public TUI entry point via the
//! backend-injection seam added in the same commit. A caller-owned
//! `ratatui::Terminal<TestBackend>` (80x24) is handed to
//! `run_with_backend` by mutable reference, scripted `TuiEvent`s are
//! pushed onto the event channel, and the rendered terminal buffer is
//! asserted to contain the expected assistant response **after** the
//! event loop exits cleanly on `TuiEvent::Done`.
//!
//! Because the test retains the `Terminal`, the buffer assertion reads
//! the very backend that `run_with_backend` drew into — not a parallel
//! snapshot. A broken `run_with_backend` that never called
//! `terminal.draw()` would leave the buffer empty and fail this test.
//!
//! The whole thing is wrapped in a 5-second `tokio::time::timeout` so
//! a misbehaving event loop never wedges CI.
//!
//! ## Why TextDelta instead of UserInput
//!
//! In the legacy `run_tui` path (the one driven by `run_with_backend`
//! for backwards compat), `TuiEvent::UserInput` is intentionally a
//! no-op — TUI-106 routed user input through `run_event_loop` instead.
//! The TUI's job is to *render* assistant output, not to dispatch the
//! LLM call. So this test injects the assistant response directly via
//! `TuiEvent::TextDelta`, mirroring what a real provider would emit.
//!
//! The hardcoded `"mock:hello"` string matches the shape a real
//! `MockProvider` would emit (`mock:<prompt>`). We don't actually
//! invoke `MockProvider` here — the assertion is "this text appears
//! in the buffer `run_with_backend` drew to", so generating the
//! string via the provider would be ceremonial. The provider is
//! exercised in its own crate's tests.

use std::time::Duration;

use archon_tui::app::{AppConfig, TuiEvent, run_with_backend};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc;

/// Helper: read every cell of a `TestBackend` buffer into a single
/// `String`, lossily flattening attributes and joining rows with `\n`.
/// Used for substring assertions on rendered output.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn app_run_drives_session_end_to_end() {
    // ── Arrange ────────────────────────────────────────────────────
    // Build the channels that AppConfig requires. The TUI consumes
    // events from event_rx and forwards user input on input_tx.
    // We don't drive input_tx in this test — the legacy run_tui path
    // doesn't process UserInput from the channel.
    let (event_tx, event_rx) = mpsc::unbounded_channel::<TuiEvent>();
    let (input_tx, _input_rx) = mpsc::channel::<String>(16);

    // The assistant response the TUI will render. Shape matches
    // `MockProvider`'s `mock:<prompt>` echo contract; hardcoded here
    // because the assertion is "this text appears in the buffer
    // `run_with_backend` drew to", not "MockProvider generated it".
    let assistant_response = "mock:hello".to_string();

    // `splash: None` tells `run_with_backend` to skip the welcome
    // screen (matches the `--bare` production contract). Without this,
    // the first frame would render the splash art instead of the
    // output buffer, and our substring assertion would target a
    // pre-event frame.
    let config = AppConfig {
        event_rx,
        input_tx,
        splash: None,
        btw_tx: None,
        permission_tx: None,
    };

    // ── Act ────────────────────────────────────────────────────────
    // Construct the Terminal locally and pass `&mut terminal` into
    // `run_with_backend`. The test retains ownership of the backend,
    // so after the loop exits we can inspect what was actually
    // rendered.
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("build TestBackend terminal");

    // Spawn a scripting task that feeds events with small delays so
    // the TUI's draw-then-recv loop has a chance to render the
    // post-TextDelta state before the `Done` event terminates it.
    //
    // The event loop's structure is: draw → drain events → check
    // should_quit → poll for key input with timeout (250ms) → loop.
    // If we push all four events up-front on the same iteration, the
    // `TextDelta` *does* mutate the `OutputBuffer` but the
    // immediately-following `Done` breaks the loop *before* the next
    // draw. The small sleeps below guarantee at least one draw lands
    // between the `TurnComplete` and `Done` events.
    //
    // We deliberately *don't* `.await` this JoinHandle — if
    // `run_with_backend` returns early (e.g. `event::poll` errors in
    // a non-tty CI environment), `event_rx` will be dropped and our
    // later `event_tx.send(...)` will fail silently (`Result` is
    // `SendError`). We would rather surface the `run_with_backend`
    // error on the main assertion than mask it with a scripter panic.
    let scripter = tokio::spawn(async move {
        let _ = event_tx.send(TuiEvent::GenerationStarted);
        let _ = event_tx.send(TuiEvent::TextDelta(String::from("mock:hello")));
        let _ = event_tx.send(TuiEvent::TurnComplete {
            input_tokens: 0,
            output_tokens: 0,
        });
        // Give the loop >= one full poll cycle (250ms) to draw the
        // post-TurnComplete frame before shutting down.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = event_tx.send(TuiEvent::Done);
    });

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_with_backend(config, &mut terminal),
    )
    .await;

    // Drain the scripter so it doesn't leak — ignore any panic, we
    // surface the underlying failure via `run_result` below.
    let _ = scripter.await;

    // ── Assert: clean exit ─────────────────────────────────────────
    let run_result = result.expect("run_with_backend timed out (>5s)");
    assert!(
        run_result.is_ok(),
        "run_with_backend returned error: {:?}",
        run_result.err()
    );

    // ── Assert: the real backend contains the response ─────────────
    // This is THE load-bearing assertion. We read the buffer owned by
    // the Terminal that `run_with_backend` drew into. If the loop
    // never called `terminal.draw()`, the buffer is empty and this
    // fails. If the loop drew but the TextDelta routing is broken,
    // "mock:hello" won't appear and this fails.
    let rendered = buffer_to_string(&terminal);
    assert!(
        rendered.contains(&assistant_response),
        "terminal backend owned by run_with_backend should contain the \
         assistant echo response '{assistant_response}' after the run; got buffer:\n{rendered}"
    );
}
