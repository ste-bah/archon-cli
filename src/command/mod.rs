//! Slash command subsystem.
//!
//! Decomposed from `src/main.rs` (TASK-AGS-621) so the slash-command
//! pipeline can be unit-tested in isolation. This module currently
//! ships only the parser. Registry (TASK-AGS-622) and dispatch
//! (TASK-AGS-623) land in later tasks.
//!
//! Declared as `mod command;` from `main.rs` so that `pub(crate)`
//! visibility scopes to the binary crate (not the library target).

#[cfg(test)]
pub(crate) static USER_DATA_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) mod add_dir;
pub(crate) mod agent;
pub(crate) mod agent_evolve;
pub(crate) mod agent_evolve_apply;
pub(crate) mod agent_evolve_apply_policy;
pub(crate) mod agent_evolve_digest;
pub(crate) mod agent_evolve_generate;
pub(crate) mod agent_evolve_generate_support;
pub(crate) mod agent_evolve_history;
pub(crate) mod agent_evolve_inspect;
pub(crate) mod agent_evolve_permissions;
pub(crate) mod agent_evolve_report;
pub(crate) mod agent_evolve_shadow;
pub(crate) mod agent_evolve_world_model;
// TASK-#211 SLASH-AGENT: /agent slash-command umbrella (list/info/run).
pub(crate) mod agent_slash;
pub(crate) mod auth;
pub(crate) mod background;
// #146: the one open-and-install body behind every board-only entry point.
pub(crate) mod board_access;
pub(crate) mod bug;
pub(crate) mod cancel;
pub(crate) mod chat;
pub(crate) mod checkpoint;
pub(crate) mod clear;
pub(crate) mod cli_mirror;
pub(crate) mod cognitive;
pub(crate) mod cognitive_adjudicate;
pub(crate) mod cognitive_daemon;
pub(crate) mod cognitive_daemon_learning;
pub(crate) mod cognitive_daemon_learning_ledger;
pub(crate) mod cognitive_view;
pub(crate) mod color;
pub(crate) mod draft;
// TASK-TUI-624: /commit AI git-commit prompt builder.
pub(crate) mod commit;
pub(crate) mod compact;
pub(crate) mod config;
// TASK-#214 SLASH-CONNECT: /connect dynamic MCP server connect.
pub(crate) mod connect;
pub(crate) mod context;
pub(crate) mod context_cmd;
pub(crate) mod copy;
pub(crate) mod cost;
pub(crate) mod denials;
pub(crate) mod diff;
pub(crate) mod dispatcher;
pub(crate) mod docs;
pub(crate) mod docs_answer;
pub(crate) mod docs_compile;
pub(crate) mod docs_delete;
#[cfg(test)]
pub(crate) mod docs_drift;
pub(crate) mod docs_embedding;
pub(crate) mod docs_index;
pub(crate) mod docs_index_daemon;
pub(crate) mod docs_index_lock;
pub(crate) mod docs_reprocess;
pub(crate) mod docs_status;
pub(crate) mod docs_vector;
pub(crate) mod doctor;
pub(crate) mod effort;
pub(crate) mod errors;
pub(crate) mod evidence_index;
pub(crate) mod evidence_view;
// TASK-#206 SLASH-EXIT: /exit handler + /q alias.
pub(crate) mod exit;
pub(crate) mod export;
// TASK-#215 SLASH-EXTRA-USAGE: /extra-usage 6-section detailed report.
pub(crate) mod extra_usage;
pub(crate) mod fast;
// TASK-#207 SLASH-FILES: /files file-picker overlay.
pub(crate) mod completion;
pub(crate) mod constellation;
pub(crate) mod files;
pub(crate) mod fork;
pub(crate) mod gametheory;
pub(crate) mod gametheory_inspect;
#[cfg(test)]
mod gametheory_inspect_tests;
pub(crate) mod gametheory_slash;
pub(crate) mod garden;
pub(crate) mod garden_adjudicate;
pub(crate) mod garden_metrics;
pub(crate) mod garden_proposals;
pub(crate) mod help;
pub(crate) mod hooks;
pub(crate) mod ide_stdio;
pub(crate) mod kb;
pub(crate) mod kb_ingest_output;
/// `kb kbs` — the only surface that enumerates knowledge-base names.
pub(crate) mod kb_kbs;
/// `kb recall` — the R7 unified recall facade's live call site.
pub(crate) mod kb_recall;
/// Real memory/docs/code-index stores behind `archon-knowledge`'s recall ports.
pub(crate) mod kb_recall_sources;
pub(crate) mod kb_reprocess;
pub(crate) mod kb_url;
pub(crate) mod learning;
pub(crate) mod learning_status;
/// What a generated workflow run feeds back to the learning stack, derived
/// from the run's own content by `archon_topology::classify_task`.
///
/// Named outside the `workflow*` prefix for the reason
/// `sona_workflow_tuning` is: `archon-topology` depends on `archon-workflow`,
/// so nothing destined for that crate may name it back.
pub(crate) mod learning_workflow_hooks;
pub(crate) mod login;
pub(crate) mod logout;
// TASK-#212 SLASH-MANAGED-AGENTS: /managed-agents remote-registry status.
pub(crate) mod managed_agents;
pub(crate) mod mcp;
pub(crate) mod meaning;
pub(crate) mod memory;
pub(crate) mod memory_cli;
pub(crate) mod model;
pub(crate) mod parser;
pub(crate) mod permissions;
pub(crate) mod permissions_cli;
pub(crate) mod pipeline;
pub(crate) mod pipeline_bundle;
pub(crate) mod pipeline_declarative;
pub(crate) mod pipeline_learning_migration;
pub(crate) mod pipeline_rewind;
pub(crate) mod pipeline_slash;
pub(crate) mod pipeline_slash_progress;
pub(crate) mod pipeline_support;
pub(crate) mod pipeline_support_result;
pub(crate) mod pipeline_workflow_llm;
pub(crate) mod provider_gate;
pub(crate) mod workflow_mcp;
// TASK-TUI-626: /plan Plan Mode toggle via SNAPSHOT+EFFECT pattern.
pub(crate) mod plan;
// TASK-P0-B.3 (#174): plan-file I/O shim (re-exports from archon_core).
pub(crate) mod plan_file;
pub(crate) mod plugin;
// TASK-#216 SLASH-PLUGIN: /plugin umbrella (list/info/hint subcommands).
pub(crate) mod plugin_slash;
// TASK-#210 SLASH-PROVIDERS: /providers list registered LLM providers.
pub(crate) mod prov;
pub(crate) mod providers;
pub(crate) mod providers_health_report;
pub(crate) mod providers_live;
pub(crate) mod providers_profile_import;
pub(crate) mod providers_slash;
pub(crate) mod providers_status;
pub(crate) mod providers_status_limits;
pub(crate) mod providers_store_cli;
// TASK-#217 SLASH-RELOAD-PLUGINS: /reload-plugins disk re-scan.
pub(crate) mod recall;
pub(crate) mod registry;
pub(crate) mod release_notes;
pub(crate) mod reload;
pub(crate) mod reload_plugins;
// TASK-#213 SLASH-REFRESH: /refresh re-scan agent registry from disk.
pub(crate) mod reasoning;
pub(crate) mod reasoning_backfill;
pub(crate) mod reasoning_label;
pub(crate) mod refresh;
pub(crate) mod remote;
pub(crate) mod rename;
/// Phase 6 traceability: `archon requirements trace`. Read-only over a PRD, a
/// task directory and an already-built code index. It never indexes — that
/// holds the Cozo write lock across a whole `multi_transaction` — and it never
/// fails a run, because an unproven requirement is a declared residual gap
/// (PRD §32), not a failure and not a pass.
pub(crate) mod requirement_trace;
pub(crate) mod resume;
// TASK-HOTFIX-V0.1.7: /run-agent primary command (#248).
/// `/archon-code` — 50-agent coding pipeline TUI primary.
pub(crate) mod archon_code;
/// `/archon-research` — 47-agent research pipeline TUI primary.
pub(crate) mod archon_research;
pub(crate) mod run_agent;
// TASK-TUI-622: /review PR code-review prompt builder.
pub(crate) mod review;
// TASK-#208 SLASH-SEARCH: /search recursive basename substring search.
pub(crate) mod search;
pub(crate) mod self_calibration;
#[cfg(test)]
mod self_calibration_tests;
// TASK-TUI-620: /rewind message-selector overlay launcher.
pub(crate) mod rewind;
pub(crate) mod rules;
// TASK-TUI-628: /sandbox handler — Bubble-mode flag flipper.
pub(crate) mod sandbox;
pub(crate) mod sandbox_cli;
pub(crate) mod sandbox_doctor;
pub(crate) mod sessions;
// TASK-TUI-625: /session remote-URL + QR code handler.
pub(crate) mod session;
// TASK-TUI-627: /skills skills-menu overlay launcher.
pub(crate) mod skills;
pub(crate) mod slash;
/// The SONA learning loop over a generated workflow run: the budget tuner, the
/// shape tuner, and the pre-run topology lint that admits a shape proposal.
///
/// Named outside the `workflow*` prefix on purpose. These read and write the
/// Cozo learning store through `archon_pipeline::learning` and
/// `topology_fold::open_store`, none of which `archon-workflow` may reach; the
/// prefix is reserved for files destined for that crate, so keeping these out
/// of it makes the boundary a one-line grep rather than a convention. The same
/// reason `pipeline_workflow_llm` is not called `workflow_pipeline_llm`.
pub(crate) mod sona_workflow_shape_gate;
pub(crate) mod sona_workflow_shape_tuning;
pub(crate) mod sona_workflow_tuning;
pub(crate) mod status;
pub(crate) mod store_paths;
pub(crate) mod style;
#[cfg(test)]
pub(crate) mod surface_matrix;
// TASK-#209 SLASH-SUMMARY: /summary one-glance session headline.
pub(crate) mod summary;
pub(crate) mod task;
pub(crate) mod team;
/// Milestone 3 topology: guardrail admission. Synchronous, in-memory only —
/// no database access of any kind, not even a read.
pub(crate) mod topology_admission;
/// Milestone 2 topology: the batched fold from ambient traces into
/// `.archon/topology.db` plus one `learning_events` summary row per graph.
/// Lives here rather than in `archon-pipeline` because the fold needs
/// `archon-workflow`, `archon-topology`, and the learning stack at once, and
/// `archon-pipeline` must not acquire an edge onto `archon-workflow`.
pub(crate) mod topology_fold;
/// Milestone 4 topology: the advisory lint suite behind `archon workflow lint`.
/// Read-only and non-blocking by construction — it loads a graph, runs pure
/// analyses, and prints. It never writes and never fails a run.
pub(crate) mod topology_lint;
/// Lowering a decomposed-PRD task directory into the topology IR, for the
/// lints and the shape gate that score one. Named outside the `workflow*`
/// prefix because it names `archon_topology`, and `archon-topology` depends on
/// `archon-workflow`.
pub(crate) mod topology_task_graph;
/// Milestone 2 topology: the ambient trace recorder. Hot path, file-only,
/// never touches a database.
pub(crate) mod topology_trace;
pub(crate) mod trading;
pub(crate) mod trading_backtest;
pub(crate) mod trading_data;
pub(crate) mod trading_data_provider;
pub(crate) mod trading_data_provider_openbb;
#[cfg(test)]
pub(crate) mod trading_data_provider_tests;
pub(crate) mod trading_io;
pub(crate) mod trading_live;
pub(crate) mod trading_openbb;
pub(crate) mod trading_paper;
pub(crate) mod trading_pine;
pub(crate) mod trading_promote;
pub(crate) mod trading_spec;
pub(crate) mod trading_tools;
pub(crate) mod trading_tv;
pub(crate) mod trading_workflow;
// TASK-TUI-623: /tag session tag toggle.
pub(crate) mod tag;
// TASK-TUI-621: hidden stub `/teleport` command (no is_visible() on
// trait — visibility handled by omission from archon-tui commands.rs).
pub(crate) mod behaviour;
/// Pins `/workflow-prd-spec`'s `tasks/PRD-<NAME>/` output location against the
/// workflow engine's directory walk — the two live in different crates.
#[cfg(test)]
mod prd_pipeline_layout_tests;
pub(crate) mod teleport;
#[cfg(any(test, feature = "test-support"))]
pub mod test_db;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub(crate) mod theme;
pub(crate) mod thinking;
pub(crate) mod tui_helpers;
pub(crate) mod tui_workflow_ui_sink;
pub(crate) mod update;
pub(crate) mod usage;
pub(crate) mod utils;
pub(crate) mod video;
pub(crate) mod video_delete;
pub(crate) mod vim;
pub(crate) mod voice;
pub(crate) mod web;
pub(crate) mod web_attach;
pub(crate) mod web_chat;
pub(crate) mod web_slash;
pub(crate) mod workflow;
pub(crate) mod workflow_capabilities;
/// What `src/command/workflow*.rs` may name, pinned as a scan rather than a
/// convention.
#[cfg(test)]
pub(crate) mod workflow_crate_boundary_tests;
pub(crate) mod workflow_live;
pub(crate) mod world_model;
pub(crate) mod world_view;

// TASK-AGS-800 (Stage 6, Q1=A): spec-name discoverability shim.
//
// The phase-8 spec (`TASK-AGS-800.md`) used the name `SlashCommand` for
// the trait. Shipped code (TASK-AGS-622) calls it `CommandHandler`.
// Stage 6 orchestrator decision Q1=A preserves the shipped trait
// verbatim (sync, `anyhow::Result<()>`, no `CommandOutcome`/`CommandError`/
// `ViewId` enums, no `inventory` registration). This re-export is a
// zero-cost namespace alias so future readers grepping for
// `SlashCommand` land on the real trait.
//
// Purely additive: no runtime behavior change, no new dependencies, no
// new types. See the TASK-AGS-800 commit body for the full R-item list.
#[allow(unused_imports)]
pub(crate) use registry::CommandHandler as SlashCommand;

// TASK-AGS-801 (Stage 6, Q1=A): parser drift-reconcile + gap-fill.
//
// Re-export the parser types so future readers grepping for
// `CommandParser` / `ParseError` / `Arg` / `suggest` land on the real
// definitions without having to dig through the `parser` submodule
// directly. This matches the additive-shim pattern established by the
// `SlashCommand` alias above and is the `mod.rs` re-export mandated by
// TASK-AGS-801 (G9).
//
// Note: `ParsedCommand` is already reachable via `parser::ParsedCommand`
// from dispatcher.rs; the re-export just widens the surface to match
// the spec's "mod.rs re-exports the 5 parser types" wiring check.
#[allow(unused_imports)]
pub(crate) use parser::{Arg, CommandParser, ParseError, ParsedCommand, suggest};
