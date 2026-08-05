//! Gate 2 real tests. Replace the Gate 1 `#[ignore]` + `todo!()`
//! skeleton with real assertions against the landed HelpHandler impl
//! and the new `CommandContext::skill_registry` field. Uses the
//! `make_help_ctx` helper added to `test_support.rs` in Gate 1. Test
//! names preserved from Gate 1 skeleton for traceability.

use super::*;
use crate::command::registry::CommandHandler;
use crate::command::test_support::*;
use archon_tui::app::TuiEvent;

#[test]
fn help_handler_empty_args_emits_core_commands_list() {
    let (mut ctx, mut rx) = make_help_ctx();
    HelpHandler.execute(&mut ctx, &[]).unwrap();
    let events = drain_tui_events(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "empty-args path must emit exactly one TextDelta; got: {:?}",
        events
    );
    match &events[0] {
        TuiEvent::TextDelta(body) => {
            assert!(
                body.contains("Core commands:"),
                "TextDelta must contain 'Core commands:' header; got: {:?}",
                body
            );
            assert!(
                body.contains("Extended commands:"),
                "TextDelta must contain 'Extended commands:' separator; got: {:?}",
                body
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta for empty-args path, got: {:?}",
            other
        ),
    }
}

#[test]
fn help_handler_empty_args_appends_skill_registry_help() {
    let (mut ctx, mut rx) = make_help_ctx();
    // Capture the expected suffix from the SAME registry the handler
    // will consult — call format_help() via the Arc stashed in ctx.
    let expected_suffix = ctx
        .skill_registry
        .as_ref()
        .map(|r| r.format_help())
        .expect("make_help_ctx populates skill_registry");
    HelpHandler.execute(&mut ctx, &[]).unwrap();
    let events = drain_tui_events(&mut rx);
    match &events[0] {
        TuiEvent::TextDelta(body) => {
            assert!(
                body.ends_with(&expected_suffix),
                "TextDelta must END with the exact output of \
                 SkillRegistry::format_help() (byte-identical \
                 suffix). Expected suffix: {:?}\nGot body: {:?}",
                expected_suffix,
                body
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta for empty-args path, got: {:?}",
            other
        ),
    }
}

#[test]
fn help_handler_with_known_command_emits_detail() {
    let (mut ctx, mut rx) = make_help_ctx();
    // `make_help_ctx` registers HelpSkill (name="help"), so
    // `format_skill_help("help")` returns Some(_).
    let expected_body = ctx
        .skill_registry
        .as_ref()
        .and_then(|r| r.format_skill_help("help"))
        .expect("make_help_ctx registers HelpSkill under name 'help'");
    HelpHandler
        .execute(&mut ctx, &[String::from("help")])
        .unwrap();
    let events = drain_tui_events(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "known-command path must emit exactly one TextDelta; got: {:?}",
        events
    );
    match &events[0] {
        TuiEvent::TextDelta(payload) => {
            // Shipped format: format!("\n{detail}\n")
            let expected_payload = format!("\n{expected_body}\n");
            assert_eq!(
                payload, &expected_payload,
                "TextDelta payload must be byte-identical to \
                 format!(\"\\n{{detail}}\\n\") from shipped slash.rs:559"
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta for known-command path, got: {:?}",
            other
        ),
    }
}

#[test]
fn help_handler_with_known_command_strips_leading_slash() {
    // Two contexts, two invocations — args=["help"] and args=["/help"]
    // must produce the SAME TextDelta payload (leading-`/` stripped
    // per shipped slash.rs:556).
    let (mut ctx_a, mut rx_a) = make_help_ctx();
    let (mut ctx_b, mut rx_b) = make_help_ctx();
    HelpHandler
        .execute(&mut ctx_a, &[String::from("help")])
        .unwrap();
    HelpHandler
        .execute(&mut ctx_b, &[String::from("/help")])
        .unwrap();
    let events_a = drain_tui_events(&mut rx_a);
    let events_b = drain_tui_events(&mut rx_b);
    assert_eq!(
        events_a.len(),
        1,
        "args=['help'] must emit one event; got: {:?}",
        events_a
    );
    assert_eq!(
        events_b.len(),
        1,
        "args=['/help'] must emit one event; got: {:?}",
        events_b
    );
    match (&events_a[0], &events_b[0]) {
        (TuiEvent::TextDelta(a), TuiEvent::TextDelta(b)) => {
            assert_eq!(
                a, b,
                "args=['help'] and args=['/help'] must emit \
                 byte-identical TextDelta payloads (leading '/' \
                 stripped per shipped slash.rs:556)"
            );
        }
        (a, b) => panic!(
            "expected both paths to emit TextDelta; got a={:?}, b={:?}",
            a, b
        ),
    }
}

#[test]
fn help_handler_with_unknown_command_emits_error() {
    let (mut ctx, mut rx) = make_help_ctx();
    HelpHandler
        .execute(&mut ctx, &[String::from("bogusname")])
        .unwrap();
    let events = drain_tui_events(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "unknown-command path must emit exactly one Error; got: {:?}",
        events
    );
    match &events[0] {
        TuiEvent::Error(msg) => {
            assert_eq!(
                msg, "Unknown command: /bogusname",
                "Error payload must be byte-identical to \
                 format!(\"Unknown command: /{{name}}\") from \
                 shipped slash.rs:563"
            );
        }
        other => panic!(
            "expected TuiEvent::Error for unknown-command path, got: {:?}",
            other
        ),
    }
}

#[test]
fn help_handler_description_byte_identical_to_shipped() {
    assert_eq!(
        HelpHandler.description(),
        "Show help for commands and shortcuts",
        "description() must be byte-identical to the shipped \
         declare_handler! macro arg at registry.rs:749"
    );
}

#[test]
fn help_handler_aliases_preserved() {
    assert_eq!(
        HelpHandler.aliases(),
        &["?", "h"],
        "aliases() must preserve the shipped registry.rs:749 alias \
         set verbatim — `?` and `h` both route to /help"
    );
}

/// Byte-identity guard for the 19-line column-aligned core-commands
/// header literal. Sherlock Gate 3 Gap 1: the original
/// `help_handler_empty_args_emits_core_commands_list` test uses
/// substring checks (`contains("Core commands:")` etc.) which do not
/// enforce column alignment, exact whitespace, `\n\n` boundaries, or
/// the terminating `"Extended commands:\n"` separator. After Gate 5
/// deletes the shipped arm at slash.rs:531-549 the suite alone must
/// catch header regressions. This test asserts byte-identity against
/// the verbatim 19-line header by exercising the empty-args path with
/// an `skill_registry: None` context (so no format_help() suffix is
/// appended) and comparing the TextDelta payload exactly.
#[test]
fn help_handler_empty_args_header_byte_identical() {
    // Build a CommandContext WITHOUT a skill registry so the handler
    // emits the core-commands header alone (no suffix). `make_status_ctx`
    // already sets skill_registry: None.
    let (mut ctx, mut rx) = make_status_ctx(None);
    HelpHandler.execute(&mut ctx, &[]).unwrap();
    let events = drain_tui_events(&mut rx);
    assert_eq!(
        events.len(),
        1,
        "empty-args path with skill_registry=None must emit exactly \
         one TextDelta; got: {:?}",
        events
    );
    let expected = "\n\
        Core commands:\n\
        /model <name>        - Switch model (opus, sonnet, haiku, or full name)\n\
        /fast                - Toggle fast mode\n\
        /effort <level>      - Set effort (high, medium, low)\n\
        /thinking on|off     - Show/hide thinking output\n\
        /compact             - Trigger context compaction\n\
        /clear               - Clear conversation history\n\
        /status              - Show current session info\n\
        /cost                - Show session cost breakdown\n\
        /permissions [mode]  - Show/set permission mode (6 modes + aliases)\n\
        /config [key] [val]  - List, get, or set runtime config values\n\
        /memory [subcmd]     - Store, list, search, or clear memories\n\
        /doctor              - Run diagnostics on all subsystems\n\
        /export              - Export conversation as JSON\n\
        /diff                - Show git diff --stat for the working directory\n\
        /help                - Show this help\n\
        /help <command>      - Show detailed help for a command\n\n\
        Extended commands:\n";
    match &events[0] {
        TuiEvent::TextDelta(body) => {
            assert_eq!(
                body, expected,
                "TextDelta payload must be BYTE-IDENTICAL to the \
                 shipped slash.rs:531-549 core-commands header. \
                 Column alignment, every \\n, every \\n\\n boundary, \
                 and the 'Extended commands:' separator MUST match."
            );
        }
        other => panic!(
            "expected TuiEvent::TextDelta for empty-args path, got: {:?}",
            other
        ),
    }
}

// -----------------------------------------------------------------
// Gate 5 live-smoke: end-to-end via real Dispatcher + default
// Registry (proves routing: dispatcher -> registry -> HelpHandler
// -> channel emission) for literal user inputs "/help", "/help help",
// "/help bogus", and the two aliases "/?" and "/h". Mirrors the
// B05-VIM dispatcher-integration harness but exercises the real
// registered HelpHandler with the real skill_registry populated.
// -----------------------------------------------------------------

#[test]
fn dispatcher_routes_slash_help_to_help_handler_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    let (mut ctx, mut rx) = make_help_ctx();

    let result = dispatcher.dispatch(&mut ctx, "/help");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/help\") must return Ok"
    );

    let events = drain_tui_events(&mut rx);
    let has_text_delta = events
        .iter()
        .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Core commands:")));
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        has_text_delta && !has_error,
        "end-to-end `/help` must emit TextDelta containing 'Core commands:' \
         AND NO Error (i.e. not routed to the unknown-command branch); \
         got: {:?}",
        events
    );
}

#[test]
fn dispatcher_routes_slash_help_with_known_command_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    let (mut ctx, mut rx) = make_help_ctx();

    // `make_help_ctx` registers HelpSkill (name="help") so
    // /help help resolves via format_skill_help to Some(_).
    let result = dispatcher.dispatch(&mut ctx, "/help help");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/help help\") must return Ok"
    );

    let events = drain_tui_events(&mut rx);
    let has_text_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        has_text_delta && !has_error,
        "end-to-end `/help help` must emit TextDelta (skill detail) \
         and NO Error; got: {:?}",
        events
    );
}

#[test]
fn dispatcher_routes_slash_help_with_unknown_command_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    let (mut ctx, mut rx) = make_help_ctx();

    let result = dispatcher.dispatch(&mut ctx, "/help bogusname");
    assert!(
        result.is_ok(),
        "dispatcher.dispatch(\"/help bogusname\") must return Ok"
    );

    let events = drain_tui_events(&mut rx);
    let has_error = events
        .iter()
        .any(|e| matches!(e, TuiEvent::Error(msg) if msg == "Unknown command: /bogusname"));
    assert!(
        has_error,
        "end-to-end `/help bogusname` must emit TuiEvent::Error with \
         byte-identical 'Unknown command: /bogusname' payload; got: {:?}",
        events
    );
}

#[test]
fn dispatcher_routes_slash_question_mark_alias_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    let (mut ctx, mut rx) = make_help_ctx();

    // `/?` MUST alias to /help and emit the core-commands header.
    let result = dispatcher.dispatch(&mut ctx, "/?");
    assert!(result.is_ok(), "dispatcher.dispatch(\"/?\") must return Ok");

    let events = drain_tui_events(&mut rx);
    let has_text_delta = events
        .iter()
        .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Core commands:")));
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        has_text_delta && !has_error,
        "end-to-end `/?` (alias) must route to HelpHandler and emit \
         TextDelta containing 'Core commands:'; got: {:?}",
        events
    );
}

#[test]
fn dispatcher_routes_slash_h_alias_end_to_end() {
    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::default_registry;
    use std::sync::Arc;

    let registry = Arc::new(default_registry());
    let dispatcher = Dispatcher::new(registry);
    let (mut ctx, mut rx) = make_help_ctx();

    // `/h` MUST alias to /help and emit the core-commands header.
    let result = dispatcher.dispatch(&mut ctx, "/h");
    assert!(result.is_ok(), "dispatcher.dispatch(\"/h\") must return Ok");

    let events = drain_tui_events(&mut rx);
    let has_text_delta = events
        .iter()
        .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Core commands:")));
    let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
    assert!(
        has_text_delta && !has_error,
        "end-to-end `/h` (alias) must route to HelpHandler and emit \
         TextDelta containing 'Core commands:'; got: {:?}",
        events
    );
}
