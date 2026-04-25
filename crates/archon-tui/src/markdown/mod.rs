//! Markdown-to-ratatui renderer.
//!
//! Relocated from `src/markdown.rs` → `src/markdown/` per REM-2c (REM-2
//! split plan, docs/rem-2-split-plan.md section 6). Zero public-API change.
//!
//! Provides two rendering modes:
//!
//! - [`render_markdown`] — Full document parser using [`pulldown_cmark`].
//!   Handles headings, bold, italic, inline code, fenced code blocks
//!   (syntax-highlighted via [`crate::syntax`]), blockquotes, lists, and links.
//!
//! - [`render_markdown_line`] — Legacy single-line renderer using theme colors.
//!   Retained for backward compatibility with the existing TUI rendering
//!   pipeline in `app.rs`.

mod legacy_line;
mod renderer;

pub use legacy_line::render_markdown_line;
pub use renderer::render_markdown;
