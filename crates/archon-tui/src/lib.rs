pub mod app;
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
pub mod pane;
pub mod pane_layout;
pub mod pane_manager;
pub mod theme_registry;
pub mod verbosity;
pub mod virtual_scroll;

#[cfg(feature = "terminal-panel")]
pub mod terminal_panel;

// Stubs for later phases
pub mod scroll {}
