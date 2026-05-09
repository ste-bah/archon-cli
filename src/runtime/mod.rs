//! Runtime composition helpers extracted from `main.rs`.
//!
//! Each submodule owns one cross-cutting construction concern so
//! `main.rs` stays a thin orchestrator. TASK-AGS-699.

pub(crate) mod agent_ledger_events;
pub(crate) mod agent_profile_overlay;
pub(crate) mod codex_provider;
pub(crate) mod codex_strategy;
pub(crate) mod llm;
pub(crate) mod permission_events;
pub(crate) mod provider_auth_selection;
pub(crate) mod provider_event_record;
pub(crate) mod provider_fallback_events;
pub(crate) mod provider_limit_windows;
pub(crate) mod provider_observer;
pub(crate) mod sandbox_events;
