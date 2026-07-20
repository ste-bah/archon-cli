use super::*;
use std::sync::Mutex;

// ---- Mock runner --------------------------------------------------

/// Test-only [`ClipboardRunner`] with configurable outcomes. The
/// three slots exercise the three terminal branches of the
/// handler (Ok / NoToolFound / SpawnFailed) deterministically.
///
/// A [`Mutex`] on `last_content` lets assertions verify the
/// exact bytes handed to the subprocess (even in the false-return
/// case — matches shipped behaviour where the subprocess spawn
/// is attempted even when `wait()` ultimately succeeds).
struct MockClipboardRunner {
    /// Tool token returned by `detect_tool()`. One of `"xclip"`,
    /// `"clip.exe"`, `"pbcopy"`, or `"none"`.
    tool: &'static str,
    /// Return value handed back by `copy_to_clipboard`. `true` →
    /// Ok branch; `false` → SpawnFailed branch.
    copy_result: bool,
    /// Captures the `content` argument of the most recent
    /// `copy_to_clipboard` call, or `None` if `detect_tool`
    /// returned `"none"` (in which case the handler skips the
    /// subprocess call).
    last_content: Mutex<Option<String>>,
}

impl MockClipboardRunner {
    fn new(tool: &'static str, copy_result: bool) -> Self {
        Self {
            tool,
            copy_result,
            last_content: Mutex::new(None),
        }
    }
}

impl ClipboardRunner for MockClipboardRunner {
    fn detect_tool(&self) -> &'static str {
        self.tool
    }

    fn copy_to_clipboard(&self, _tool: &str, content: &str) -> bool {
        *self.last_content.lock().unwrap() = Some(content.to_string());
        self.copy_result
    }
}

// ---- make_ctx -----------------------------------------------------

/// Build a `CommandContext` with a freshly-created channel and an
/// optional [`CopySnapshot`]. All other optional fields stay
/// `None`. Mirrors the make_ctx fixtures in permissions.rs /
/// effort.rs / add_dir.rs.
fn make_ctx(
    snapshot: Option<CopySnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_copy_snapshot_opt(snapshot)
        .build()
}

/// Drain `rx` non-blockingly into a Vec — matches the `drain`
/// helper in permissions.rs / effort.rs test modules.
fn drain(rx: &mut archon_tui::event_channel::TuiEventReceiver) -> Vec<TuiEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

// ---- R1: description + aliases byte-identity tests ----------------

#[test]
fn copy_handler_description_byte_identical_to_shipped() {
    let h = CopyHandler::new();
    assert_eq!(
        h.description(),
        "Copy the last assistant message to the clipboard",
        "CopyHandler description must match the shipped \
         declare_handler! stub at registry.rs:1014 byte-for-byte \
         (shipped-wins drift-reconcile)"
    );
}

#[test]
fn copy_handler_aliases_are_empty() {
    let h = CopyHandler::new();
    assert_eq!(
        h.aliases(),
        &[] as &[&'static str],
        "CopyHandler aliases must be empty to match the shipped \
         declare_handler! stub (two-arg form, no aliases slice)"
    );
}

// ---- R2: snapshot-missing Err branch ------------------------------

#[test]
fn copy_handler_execute_without_snapshot_returns_err() {
    let (mut ctx, _rx) = make_ctx(None);
    let h = CopyHandler::new();
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_err(),
        "CopyHandler::execute must return Err when copy_snapshot \
         is None (builder contract violation), got: {res:?}"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.to_lowercase().contains("copy_snapshot"),
        "Err message must mention 'copy_snapshot' for operator \
         traceability, got: {msg}"
    );
    assert!(
        msg.contains("wiring") || msg.contains("builder"),
        "Err message must mention 'wiring' or 'builder' to locate \
         the fix site, got: {msg}"
    );
}

// ---- Empty-response branch ----------------------------------------

#[test]
fn copy_handler_execute_empty_last_response_emits_textdelta() {
    let snap = CopySnapshot {
        last_response: String::new(),
    };
    // NoToolFound mock to prove the subprocess call is skipped
    // BEFORE tool detection — empty-branch must short-circuit.
    let runner = Arc::new(MockClipboardRunner::new("none", false));
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = CopyHandler::with_runner(runner.clone());
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "empty-response branch must return Ok (emission via \
         TuiEvent), got: {res:?}"
    );

    // Exactly one TextDelta event.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "empty-response branch must emit exactly one event; \
         got: {events:?}"
    );
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, "\nNo assistant response to copy.\n",
                "empty-response TextDelta must match shipped \
                 slash.rs:156-160 byte-for-byte"
            );
        }
        other => panic!(
            "empty-response must emit TuiEvent::TextDelta, got: \
             {other:?}"
        ),
    }

    // Mock runner must NOT have been asked to copy — empty
    // branch skips the subprocess entirely.
    assert!(
        runner.last_content.lock().unwrap().is_none(),
        "empty-response branch must NOT invoke copy_to_clipboard"
    );
}

// ---- Ok branch: successful clipboard copy -------------------------

#[test]
fn copy_handler_execute_ok_tool_emits_copied_chars_textdelta() {
    let response = "hello world".to_string();
    let chars = response.len();
    let snap = CopySnapshot {
        last_response: response.clone(),
    };
    let runner = Arc::new(MockClipboardRunner::new("xclip", true));
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = CopyHandler::with_runner(runner.clone());
    let res = h.execute(&mut ctx, &[]);
    assert!(res.is_ok(), "ok-tool branch must return Ok, got: {res:?}");

    // Exactly one TextDelta with the success format.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "ok-tool branch must emit exactly one event; got: {events:?}"
    );
    let expected = format!("\nCopied {chars} characters to clipboard.\n");
    match &events[0] {
        TuiEvent::TextDelta(text) => {
            assert_eq!(
                text, &expected,
                "ok-tool TextDelta must match format!(\"\\nCopied \
                 {{chars}} characters to clipboard.\\n\") \
                 byte-for-byte"
            );
        }
        other => panic!("ok-tool must emit TuiEvent::TextDelta, got: {other:?}"),
    }

    // Runner received the exact response bytes.
    assert_eq!(
        runner.last_content.lock().unwrap().as_deref(),
        Some(response.as_str()),
        "copy_to_clipboard must receive the snapshot's \
         last_response byte-for-byte"
    );
}

// ---- NoToolFound branch -------------------------------------------

#[test]
fn copy_handler_execute_no_tool_emits_error() {
    let snap = CopySnapshot {
        last_response: "non-empty".to_string(),
    };
    let runner = Arc::new(MockClipboardRunner::new("none", false));
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = CopyHandler::with_runner(runner.clone());
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "no-tool branch must still return Ok (error emitted via \
         TuiEvent::Error, not surfaced via Err), got: {res:?}"
    );

    // Exactly one Error event.
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "no-tool branch must emit exactly one event; got: {events:?}"
    );
    match &events[0] {
        TuiEvent::Error(text) => {
            assert_eq!(
                text,
                "No clipboard tool found. Install xclip (Linux), or use clip.exe (WSL) / pbcopy (macOS).",
                "no-tool Error must match shipped slash.rs:247-249 \
                 byte-for-byte"
            );
        }
        other => panic!("no-tool must emit TuiEvent::Error, got: {other:?}"),
    }

    // Runner must NOT have been asked to copy — handler skips
    // subprocess when detect_tool == "none".
    assert!(
        runner.last_content.lock().unwrap().is_none(),
        "no-tool branch must NOT invoke copy_to_clipboard"
    );
}

// ---- SpawnFailed branch -------------------------------------------

#[test]
fn copy_handler_execute_spawn_failed_emits_error() {
    let snap = CopySnapshot {
        last_response: "some content".to_string(),
    };
    // Detect returns a real tool, but copy_to_clipboard returns
    // false — simulates a spawn failure (e.g., tool was in PATH
    // at `which` time but vanished before spawn, or `spawn()`
    // returned Err for any reason).
    let runner = Arc::new(MockClipboardRunner::new("xclip", false));
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let h = CopyHandler::with_runner(runner.clone());
    let res = h.execute(&mut ctx, &[]);
    assert!(
        res.is_ok(),
        "spawn-failed branch must still return Ok, got: {res:?}"
    );

    // Exactly one Error event — same byte-identical message as
    // the no-tool branch (shipped semantics: ANY copied==false
    // routes to the same Error string).
    let events = drain(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "spawn-failed branch must emit exactly one event; got: \
         {events:?}"
    );
    match &events[0] {
        TuiEvent::Error(text) => {
            assert_eq!(
                text,
                "No clipboard tool found. Install xclip (Linux), or use clip.exe (WSL) / pbcopy (macOS).",
                "spawn-failed Error must match the no-tool Error \
                 byte-for-byte (shipped routes both to the same \
                 string via `copied == false`)"
            );
        }
        other => panic!("spawn-failed must emit TuiEvent::Error, got: {other:?}"),
    }

    // Runner SHOULD have been asked (detect_tool returned real
    // tool, so handler did invoke copy_to_clipboard).
    assert_eq!(
        runner.last_content.lock().unwrap().as_deref(),
        Some("some content"),
        "spawn-failed branch must still pipe content to \
         copy_to_clipboard (shipped invokes spawn before wait)"
    );
}

// ---- Gate 5: Dispatcher-integration tests ------------------------
//
// Mirror B13-GARDEN / B12-PERMISSIONS Gate 5 precedent. Build a
// REAL `Arc<Registry>` + `Dispatcher::new` — not `default_registry()`
// (which hard-wires `SystemClipboardRunner` at registry.rs:1222
// and would hit a real xclip / clip.exe / pbcopy binary). Instead,
// construct a narrow `RegistryBuilder` with a `CopyHandler` wired
// to `MockClipboardRunner` so the subprocess outcome is
// deterministic. This verifies:
//   1. Dispatcher routes "/copy" to CopyHandler (registry key
//      resolution working — i.e., insert_primary at registry.rs:1222
//      is alive and the alias map is wired through build()).
//   2. CopySnapshot threading from context wiring is observed by
//      the handler (test supplies snapshot via make_ctx — the
//      `build_command_context` path is exercised in live smoke).
//   3. Empty-response short-circuit fires BEFORE subprocess
//      detection (first test).
//   4. Ok-tool success emits the shipped-format TextDelta via the
//      dispatcher round-trip (second test).
//   5. NO CommandEffect stashed (SNAPSHOT pattern — write side is
//      out-of-process, not a CommandEffect mutex write).
//   6. NO TuiEvent::Error on the happy paths.

#[test]
fn dispatcher_routes_slash_copy_with_empty_response_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    // Build a narrow registry with ONLY the /copy primary wired
    // to a mock runner. Default_registry is intentionally NOT
    // used — it would wire SystemClipboardRunner which hits real
    // clipboard binaries on the test host.
    let runner = Arc::new(MockClipboardRunner::new("none", false));
    let mut builder = RegistryBuilder::new();
    builder.insert_primary("copy", Arc::new(CopyHandler::with_runner(runner.clone())));
    let registry = Arc::new(builder.build());
    let dispatcher = Dispatcher::new(registry);

    // Empty last_response → handler short-circuits BEFORE tool
    // detection. This is a deterministic route.
    let snap = CopySnapshot {
        last_response: String::new(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let result = dispatcher.dispatch(&mut ctx, "/copy");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/copy\") with empty snapshot must \
         return Ok; got: {result:?}"
    );

    // 1. NO pending_effect (SNAPSHOT pattern — write side is out-
    //    of-process subprocess spawn, not a mutex write).
    assert!(
        ctx.pending_effect.is_none(),
        "end-to-end `/copy` must NOT stash a CommandEffect \
         (SNAPSHOT-pattern invariant); got: {:?}",
        ctx.pending_effect
    );

    // 2. Exactly one TextDelta byte-identical to shipped
    //    slash.rs:156-160 (pre-arm-delete reference).
    let mut got: Option<String> = None;
    let mut has_error = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            TuiEvent::TextDelta(text) => got = Some(text),
            TuiEvent::Error(_) => has_error = true,
            _ => {}
        }
    }
    let text = got.expect(
        "end-to-end `/copy` with empty snapshot must emit a \
         TuiEvent::TextDelta",
    );
    assert_eq!(
        text, "\nNo assistant response to copy.\n",
        "end-to-end `/copy` empty-response TextDelta must match \
         shipped byte-for-byte"
    );

    // 3. NO Error event on the happy path.
    assert!(
        !has_error,
        "end-to-end `/copy` with empty snapshot must emit NO \
         TuiEvent::Error"
    );

    // 4. Mock runner NEVER invoked — empty branch short-circuits.
    assert!(
        runner.last_content.lock().unwrap().is_none(),
        "end-to-end `/copy` empty-response branch must NOT invoke \
         copy_to_clipboard on the runner"
    );
}

#[test]
fn dispatcher_routes_slash_copy_with_ok_tool_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    let response = "dispatcher-integration content".to_string();
    let chars = response.len();
    let runner = Arc::new(MockClipboardRunner::new("xclip", true));
    let mut builder = RegistryBuilder::new();
    builder.insert_primary("copy", Arc::new(CopyHandler::with_runner(runner.clone())));
    let registry = Arc::new(builder.build());
    let dispatcher = Dispatcher::new(registry);

    let snap = CopySnapshot {
        last_response: response.clone(),
    };
    let (mut ctx, mut rx) = make_ctx(Some(snap));
    let result = dispatcher.dispatch(&mut ctx, "/copy");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/copy\") with ok-tool snapshot must \
         return Ok; got: {result:?}"
    );

    // 1. NO pending_effect (SNAPSHOT pattern).
    assert!(
        ctx.pending_effect.is_none(),
        "end-to-end `/copy` must NOT stash a CommandEffect; got: {:?}",
        ctx.pending_effect
    );

    // 2. Exactly one TextDelta byte-identical to shipped
    //    slash.rs:241-245 format!("\nCopied {chars} characters to
    //    clipboard.\n").
    let mut got: Option<String> = None;
    let mut has_error = false;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            TuiEvent::TextDelta(text) => got = Some(text),
            TuiEvent::Error(_) => has_error = true,
            _ => {}
        }
    }
    let text = got.expect(
        "end-to-end `/copy` with ok-tool snapshot must emit a \
         TuiEvent::TextDelta",
    );
    let expected = format!("\nCopied {chars} characters to clipboard.\n");
    assert_eq!(
        text, expected,
        "end-to-end `/copy` ok-tool TextDelta must match shipped \
         format!(\"\\nCopied {{chars}} characters to clipboard.\\n\") \
         byte-for-byte"
    );

    // 3. NO Error event.
    assert!(
        !has_error,
        "end-to-end `/copy` with ok-tool snapshot must emit NO \
         TuiEvent::Error"
    );

    // 4. Mock runner received the exact response bytes — proves
    //    CopySnapshot::last_response threads through the handler
    //    into the subprocess call.
    assert_eq!(
        runner.last_content.lock().unwrap().as_deref(),
        Some(response.as_str()),
        "end-to-end `/copy` must pass CopySnapshot::last_response \
         to copy_to_clipboard byte-for-byte"
    );
}
