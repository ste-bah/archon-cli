//! Tests for the terminal tools (#189 Phase 6).
//!
//! The live-shell behaviour is covered in `terminal_registry_tests`. What is
//! left here is the model-facing contract: what the arguments mean, what the
//! errors say, and — the part that matters most — that writing into a terminal
//! is gated exactly as hard as running the same text through `Bash`.

use super::*;

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "terminal-tool-tests".to_string(),
        ..Default::default()
    }
}

fn writer() -> TerminalWriteTool {
    TerminalWriteTool {
        safe_commands: vec!["ls".to_string()],
        risky_commands: vec!["cargo".to_string()],
        dangerous_commands: vec!["rm -rf".to_string()],
    }
}

#[test]
fn the_create_schema_offers_the_shells_that_exist_and_names_the_default() {
    let schema = TerminalCreateTool.input_schema();
    let shells = schema["properties"]["shell"]["enum"]
        .as_array()
        .expect("shell is an enum");

    assert!(shells.iter().any(|value| value == shells::default_shell()));
    let described = schema["properties"]["shell"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(described.contains(shells::default_shell()), "{described}");
}

/// A command the operator marked dangerous must stay dangerous when it is
/// typed into a terminal instead of passed to `Bash`. Otherwise this tool is a
/// way around the gate rather than a way to keep a shell open.
#[test]
fn a_dangerous_command_is_still_dangerous_through_a_terminal() {
    let level = writer().permission_level(&json!({"id": "t", "text": "rm -rf /srv"}));

    assert_eq!(level, PermissionLevel::Dangerous);
}

/// The floor. `ls` is on the operator's safe list, and through `Bash` it would
/// run unprompted — but this call also carries whatever state the shell is
/// already in, which the classifier cannot see.
#[test]
fn a_safe_listed_command_is_still_floored_at_risky() {
    let level = writer().permission_level(&json!({"id": "t", "text": "ls"}));

    assert_eq!(level, PermissionLevel::Risky);
}

#[test]
fn an_ordinary_command_is_risky() {
    assert_eq!(
        writer().permission_level(&json!({"id": "t", "text": "cargo build"})),
        PermissionLevel::Risky
    );
}

#[test]
fn opening_and_reading_are_gated_differently() {
    let nothing = json!({});
    // Opening a shell runs startup files; reading back what this agent already
    // caused has no effect of its own.
    assert_eq!(
        TerminalCreateTool.permission_level(&nothing),
        PermissionLevel::Risky
    );
    assert_eq!(
        TerminalReadTool.permission_level(&nothing),
        PermissionLevel::Safe
    );
    assert_eq!(
        TerminalCloseTool.permission_level(&nothing),
        PermissionLevel::Safe
    );
}

#[test]
fn the_declared_effects_match_what_each_tool_can_do() {
    assert_eq!(
        TerminalCreateTool.working_tree_effect(),
        WorkingTreeEffect::Arbitrary
    );
    assert_eq!(
        writer().working_tree_effect(),
        WorkingTreeEffect::Arbitrary,
        "a terminal write runs commands"
    );
    assert_eq!(
        TerminalReadTool.working_tree_effect(),
        WorkingTreeEffect::None
    );
    assert_eq!(
        TerminalCloseTool.working_tree_effect(),
        WorkingTreeEffect::None
    );
}

#[tokio::test]
async fn an_unknown_id_says_why_it_might_be_gone() {
    let result = TerminalReadTool
        .execute(json!({"id": "term-nope"}), &ctx())
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("idle"), "{}", result.content);
    assert!(
        result.content.contains("TerminalCreate"),
        "the message must say what to do instead: {}",
        result.content
    );
}

#[tokio::test]
async fn writing_without_a_target_or_text_is_refused() {
    let no_id = writer().execute(json!({"text": "ls"}), &ctx()).await;
    assert!(no_id.is_error);
    assert!(no_id.content.contains("id"), "{}", no_id.content);

    let no_text = writer().execute(json!({"id": "term-x"}), &ctx()).await;
    assert!(no_text.is_error);
    assert!(no_text.content.contains("text"), "{}", no_text.content);
}

#[tokio::test]
async fn an_unknown_shell_is_refused_before_anything_is_registered() {
    let result = TerminalCreateTool
        .execute(json!({"shell": "fish"}), &ctx())
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("fish"), "{}", result.content);
}

/// Closing a session's terminals is the way out, and it must not blow up when
/// there is nothing to close — every session end calls it.
#[test]
fn closing_the_terminals_of_a_session_that_opened_none_is_zero() {
    assert_eq!(close_session_terminals("session-that-never-opened-one"), 0);
}

mod under_a_sandbox {
    //! What the tools do once a backend holds the execution world (#201
    //! Phase 6).

    use std::sync::Arc;

    use archon_permissions::sandbox::SandboxTerminalCommand;

    use super::*;
    use crate::terminal_world::tests::FixedTerminalBackend;

    fn sandboxed_ctx(
        session: &str,
        backend: Arc<dyn archon_permissions::SandboxBackend>,
    ) -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: session.to_string(),
            sandbox: Some(backend),
            ..Default::default()
        }
    }

    /// A door that really runs, so the assertion is about what came out of the
    /// PTY rather than about what the plan intended to spawn.
    fn echoing_door(marker: &str) -> SandboxTerminalCommand {
        let (program, args) = if cfg!(windows) {
            ("cmd.exe", vec!["/c".to_string(), format!("echo {marker}")])
        } else {
            ("/bin/sh", vec!["-c".to_string(), format!("echo {marker}")])
        };
        SandboxTerminalCommand {
            program: program.to_string(),
            args,
            shell: "bash".into(),
            location: "/workspace in the test container".into(),
        }
    }

    #[tokio::test]
    async fn a_backend_that_refuses_terminals_leaves_nothing_running() {
        let session = "terminal-sandbox-refused";
        let ctx = sandboxed_ctx(
            session,
            FixedTerminalBackend::refusing("openshell sandbox: no session to attach to"),
        );

        let result = TerminalCreateTool.execute(json!({}), &ctx).await;

        assert!(result.is_error, "{}", result.content);
        assert!(
            result.content.contains("no session to attach to"),
            "the refusal must say why: {}",
            result.content
        );
        assert!(
            crate::terminal_registry::ids_for_session(session).is_empty(),
            "a refused terminal must not be registered"
        );
    }

    /// The whole phase in one test: with a backend holding the world, the
    /// process on the end of the PTY is the backend's, not a host shell.
    #[tokio::test]
    #[serial_test::serial]
    async fn the_shell_that_opens_is_the_one_the_backend_named() {
        let session = "terminal-sandbox-open";
        let marker = "ARCHON-SANDBOX-DOOR-OPENED";
        let ctx = sandboxed_ctx(session, FixedTerminalBackend::opening(echoing_door(marker)));

        let created = TerminalCreateTool.execute(json!({}), &ctx).await;
        assert!(!created.is_error, "{}", created.content);
        assert!(
            created.content.contains("/workspace in the test container"),
            "the model must be told where its shell actually is: {}",
            created.content
        );

        let id = created
            .content
            .split_whitespace()
            .find(|word| word.starts_with("term-"))
            .expect("the id is in the reply")
            .to_string();

        let mut seen = String::new();
        for _ in 0..100 {
            let read = TerminalReadTool
                .execute(json!({"id": id, "since": 0}), &ctx)
                .await;
            seen = read.content;
            if seen.contains(marker) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let _ = TerminalCloseTool.execute(json!({"id": id}), &ctx).await;

        assert!(
            seen.contains(marker),
            "the host shell ran instead of the backend's command: {seen}"
        );
    }

    /// Turning a sandbox on does not move a running host shell into it, so the
    /// shell that was already open becomes a way to run commands outside the
    /// boundary unless writing to it is refused.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_host_terminal_cannot_be_written_to_once_a_backend_holds_the_world() {
        let session = "terminal-sandbox-toggled-on";
        let host_ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: session.to_string(),
            ..Default::default()
        };
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let created = TerminalCreateTool
            .execute(json!({"shell": shell}), &host_ctx)
            .await;
        assert!(!created.is_error, "{}", created.content);
        let id = created
            .content
            .split_whitespace()
            .find(|word| word.starts_with("term-"))
            .expect("the id is in the reply")
            .to_string();

        let refused = writer()
            .execute(
                json!({"id": id, "text": "whoami"}),
                &sandboxed_ctx(
                    session,
                    FixedTerminalBackend::opening(echoing_door("unused")),
                ),
            )
            .await;
        let _ = TerminalCloseTool
            .execute(json!({"id": id}), &host_ctx)
            .await;

        assert!(refused.is_error, "{}", refused.content);
        assert!(
            refused.content.contains("outside the sandbox"),
            "{}",
            refused.content
        );
    }
}

/// End to end through the tools, on a real shell: open, write, read back.
#[tokio::test]
#[serial_test::serial]
async fn a_terminal_opened_by_the_tool_runs_what_is_written_to_it() {
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let created = TerminalCreateTool
        .execute(json!({"shell": shell}), &ctx())
        .await;
    assert!(!created.is_error, "{}", created.content);

    let id = created
        .content
        .split_whitespace()
        .find(|word| word.starts_with("term-"))
        .expect("the id is in the reply")
        .to_string();

    // `cd` / `pwd` prints something the command itself does not contain, so
    // finding it proves the shell ran rather than merely echoed the input.
    let show = if cfg!(windows) { "cd" } else { "pwd" };
    let written = writer()
        .execute(json!({"id": id, "text": show}), &ctx())
        .await;
    assert!(!written.is_error, "{}", written.content);

    let expected = std::env::temp_dir();
    let expected = expected
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string();
    let mut seen = String::new();
    for _ in 0..200 {
        let read = TerminalReadTool
            .execute(json!({"id": id, "since": 0}), &ctx())
            .await;
        seen = read.content;
        if seen.contains(&expected) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let closed = TerminalCloseTool.execute(json!({"id": id}), &ctx()).await;
    assert!(!closed.is_error, "{}", closed.content);

    assert!(seen.contains("next_offset:"), "{seen}");
    assert!(
        seen.contains(&expected),
        "the shell never reported its directory, so nothing ran: {seen}"
    );
}
