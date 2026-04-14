pub mod app;
pub use app::should_process_key_event;
pub mod commands;
pub mod input;
pub mod markdown;
pub mod output;
pub mod permissions;
pub mod splash;
pub mod status;
pub mod theme;
pub mod ultrathink;
pub mod vim;
pub mod voice;

pub mod syntax;

pub mod diff_view;
pub mod theme_registry;
pub mod verbosity;
pub mod virtual_scroll;

#[cfg(feature = "terminal-panel")]
pub mod terminal_panel;

pub mod split_pane;

pub mod task_dispatch;
pub use task_dispatch::{
    AgentDispatcher, AgentRouter, CancelOutcome, DispatchResult, QueuedPrompt, TurnOutcome,
    TurnRunner,
};

pub mod layout;
pub use layout::{ReflowOutcome, handle_resize, last_known_size};

pub mod views;

// Stubs for later phases
pub mod scroll {}
