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
pub mod observability;
pub use task_dispatch::{
    AgentDispatcher, AgentRouter, CancelOutcome, DispatchResult, QueuedPrompt, TurnOutcome,
    TurnRunner,
};

pub mod layout;
pub use layout::{ReflowOutcome, handle_resize, last_known_size};

pub mod event_loop;
pub use event_loop::{EventLoopConfig, run_event_loop};

pub mod cancel;
pub use cancel::YieldGate;

pub mod views;

pub mod events;
pub use events::TuiEvent;
pub mod state;
pub use state::AppState;
pub mod terminal;
pub mod keybindings;
pub mod render;
pub mod prompt_input;
pub mod overlays;
pub mod virtual_list;
pub mod message_renderer;
pub mod notifications;
pub mod context_viz;
pub mod screens;

// Stubs for later phases
pub mod scroll {}
