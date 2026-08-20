pub mod app;
pub mod app_modals;
mod app_task_overlay;
mod app_views;
pub use app::should_process_key_event;
pub mod activity_stream;
mod activity_stream_helpers;
mod activity_stream_layout;
pub mod agent_activity;
// TUI-327: re-export public TUI entry points so integration tests and
// downstream callers can pick the right one without reaching into `app::`.
// `run` is the production crossterm path; `run_with_backend` is the
// backend-injection seam for headless tests.
pub use app::{run, run_with_backend};
pub mod commands;
pub mod context_status;
pub mod input;
pub mod markdown;
pub mod output;
/// Shared modal-overlay chrome. Crate-private: it is house style, not API.
mod overlay;
pub mod permissions;
mod thinking_archive;
#[cfg(test)]
mod thinking_archive_tests;
mod thinking_view;
mod tool_transcript;
#[cfg(test)]
mod tool_transcript_tests;
// TASK-TUI-628: sandbox module — logical Bubble-mode permission check.
pub mod sandbox;
pub mod splash;
pub mod splash_compat;
mod splash_image;
pub mod status;
pub mod theme;
pub mod ultrathink;
pub mod vim;
pub mod voice;

pub mod syntax;

// TASK-TUI-625: QR rendering helper, encapsulates the `qrcode` crate.
pub mod qr;

pub mod diff_view;
pub mod theme_registry;
pub mod verbosity;
pub mod video_events;
pub mod virtual_scroll;

#[cfg(feature = "terminal-panel")]
pub mod terminal_panel;

pub mod split_pane;

pub mod observability;
pub mod observability_tracing;
pub mod task_dispatch;
pub use task_dispatch::{
    AgentDispatcher, AgentRouter, CancelOutcome, DispatchResult, QueuedPrompt, TurnOutcome,
    TurnRunner,
};

pub mod layout;
pub use layout::{ReflowOutcome, handle_resize, last_known_size};

pub mod event_loop;

pub mod cancel;
pub use cancel::YieldGate;

/// TUI entry points, split out of `app.rs` for the 500-line gate.
mod app_run;
pub mod event_channel;
mod event_framing;
mod event_payload_size;
mod event_queue_metrics;
pub mod events;
/// Agent-activity payload types, split out of `events.rs` for the 500-line
/// gate. Re-exported from `events`, so no caller changes.
pub mod events_activity;
/// `TuiEvent::variant_name`, split out of `events.rs` for the same reason: one
/// arm per variant, growing with every event and describing none of them.
pub mod events_variant_name;
pub mod evidence_view_state;
pub use events::TuiEvent;
pub mod state;
pub use state::AppState;
pub mod context_viz;
pub mod keybindings;
pub mod keylog;
pub mod message_renderer;
pub mod notifications;
pub mod overlays;
pub mod prompt_input;
pub mod render;
pub mod screens;
/// The one screen item constructed from outside this crate.
///
/// The `screens` sub-modules are crate-private so that an unwired screen trips
/// `dead_code` instead of passing CI (see `screens/mod.rs`). Naming the single
/// genuine export here keeps that property; widening the module back to `pub`
/// would restore the hole.
pub use screens::file_picker::walker::read_dir_entries;
/// The tasks-overlay seam (#189 Phase 9).
///
/// The overlay itself stays crate-private — `App` constructs it. These three
/// are re-exported because the binary implements `TaskStore` over
/// `archon_tools::task_manager::TASK_MANAGER`, which this crate cannot reach.
pub use screens::task_overlay::{TaskId, TaskRow, TaskStore};
pub mod terminal;
pub mod trading;
pub mod virtual_list;

// Stubs for later phases
pub mod scroll {}
