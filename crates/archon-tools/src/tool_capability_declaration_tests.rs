//! What this crate's tools declare about the world they act on (#201 Phase 3).
//!
//! The compiler proves every tool declares *something*. These assert the
//! declarations are the right ones, because a wrong class is exactly as
//! dangerous as no class: `Write` declaring `HostLocal` would sail through a
//! sandbox and mutate the host tree.

use crate::tool::{Tool, ToolCapability};

fn declared() -> Vec<(Box<dyn Tool>, ToolCapability)> {
    vec![
        // Reads of the world's files.
        (
            Box::new(crate::file_read::ReadTool),
            ToolCapability::FILE_READ,
        ),
        (
            Box::new(crate::glob_tool::GlobTool),
            ToolCapability::FILE_READ,
        ),
        (Box::new(crate::grep::GrepTool), ToolCapability::FILE_READ),
        (
            Box::new(crate::cartographer::CartographerTool),
            ToolCapability::FILE_READ,
        ),
        // Writes to the world's files.
        (
            Box::new(crate::file_write::WriteTool),
            ToolCapability::FILE_WRITE,
        ),
        (
            Box::new(crate::file_edit::EditTool),
            ToolCapability::FILE_WRITE,
        ),
        (
            Box::new(crate::notebook::NotebookEditTool),
            ToolCapability::FILE_WRITE,
        ),
        (
            Box::new(crate::apply_patch::ApplyPatchTool),
            ToolCapability::FILE_WRITE,
        ),
        (
            Box::new(crate::large_edit::LargeEditCommitTool),
            ToolCapability::FILE_WRITE,
        ),
        // The one seam every backend implements.
        (
            Box::new(crate::bash::BashTool::default()),
            ToolCapability::EXECUTION,
        ),
        // Reaches the world by a route no backend can redirect.
        (
            Box::new(crate::monitor::MonitorTool),
            ToolCapability::HOST_HANDLE,
        ),
        (
            Box::new(crate::terminal_tools::TerminalCreateTool),
            ToolCapability::HOST_HANDLE,
        ),
        (
            Box::new(crate::terminal_tools::TerminalReadTool),
            ToolCapability::HOST_HANDLE,
        ),
        (
            Box::new(crate::worktree::EnterWorktreeTool),
            ToolCapability::HOST_HANDLE,
        ),
        (
            Box::new(crate::java::JavaToolchain),
            ToolCapability::HOST_HANDLE,
        ),
        (
            Box::new(crate::docs::DocSearch),
            ToolCapability::HOST_HANDLE,
        ),
        // Archon's own state.
        (Box::new(crate::sleep::SleepTool), ToolCapability::HostLocal),
        (
            Box::new(crate::todo_write::TodoWriteTool),
            ToolCapability::HostLocal,
        ),
        (
            Box::new(crate::ask_user::AskUserTool),
            ToolCapability::HostLocal,
        ),
        (
            Box::new(crate::plan_mode::EnterPlanModeTool),
            ToolCapability::HostLocal,
        ),
        (
            Box::new(crate::task_get::TaskGetTool),
            ToolCapability::HostLocal,
        ),
        // Off the machine.
        (
            Box::new(crate::webfetch::WebFetchTool),
            ToolCapability::Egress,
        ),
        (
            Box::new(crate::web_search::WebSearchTool),
            ToolCapability::Egress,
        ),
        (
            Box::new(crate::push_notification::PushNotificationTool),
            ToolCapability::Egress,
        ),
        // Spawns or schedules work.
        (
            Box::new(crate::task_create::TaskCreateTool),
            ToolCapability::ControlPlane,
        ),
        (
            Box::new(crate::task_stop::TaskStopTool),
            ToolCapability::ControlPlane,
        ),
    ]
}

#[test]
fn each_tool_declares_the_class_its_effects_land_in() {
    for (tool, expected) in declared() {
        assert_eq!(
            tool.capability(),
            expected,
            "{} declares {}, expected {}",
            tool.name(),
            tool.capability().label(),
            expected.label()
        );
    }
}

/// A sweep that gave every tool one convenient class would satisfy the compiler
/// and defeat the point. Requiring all four to appear makes that visible.
#[test]
fn the_declarations_use_every_class() {
    let mut world_bound = false;
    let mut host_local = false;
    let mut egress = false;
    let mut control_plane = false;

    for (tool, _) in declared() {
        match tool.capability() {
            ToolCapability::WorldBound(_) => world_bound = true,
            ToolCapability::HostLocal => host_local = true,
            ToolCapability::Egress => egress = true,
            ToolCapability::ControlPlane => control_plane = true,
        }
    }

    assert!(world_bound && host_local && egress && control_plane);
}

/// The document and governed-learning tools read as Archon's own state and the
/// issue's own table lists `DocSearch` as host-local — but every one of them
/// runs the `archon` executable as a host subprocess through
/// `evidence_cli::run_archon`. Under isolation that subprocess escapes the
/// sandbox, so they are host handles. This is the assertion that would break if
/// someone later "corrected" them to `HostLocal` by name.
#[test]
fn cli_backed_evidence_tools_are_host_handles_not_host_local() {
    for tool in [
        Box::new(crate::docs::DocList) as Box<dyn Tool>,
        Box::new(crate::docs::DocGet),
        Box::new(crate::docs::DocSearch),
        Box::new(crate::docs::DocAnswer),
        Box::new(crate::learning::LearningStatus),
        Box::new(crate::learning::BehaviourApprove),
    ] {
        assert_eq!(
            tool.capability(),
            ToolCapability::HOST_HANDLE,
            "{} would run a host process under a sandbox",
            tool.name()
        );
    }
}
