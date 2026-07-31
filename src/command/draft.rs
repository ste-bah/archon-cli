//! `archon draft` — run the FCDP drafting protocol on a context pack.
//!
//! Surfaces the archon-draft orchestrator (D1 → D1.5 → D2 → gauntlet → R-loop) as a
//! first-class subcommand. Model resolves via `--model` → configured Anthropic Opus →
//! built-in default; auth (subscription OAuth or API key) is resolved by archon-llm.
//!
//! The orchestrator is synchronous and its model client drives its own Tokio runtime, so
//! it is run on a blocking thread (`spawn_blocking`) — a `block_on` inside the async
//! main runtime would panic.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_draft::fable::{self, FableClient};
use archon_draft::{GateConfig, Pack, QuoteBank, orchestrator};
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

include!("draft_a.rs");
include!("draft_b.rs");
