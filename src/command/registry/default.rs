use std::sync::Arc;

use super::{Registry, RegistryBuilder};
use crate::command::add_dir::AddDirHandler;
use crate::command::archon_code::ArchonCodeHandler;
use crate::command::archon_research::ArchonResearchHandler;
use crate::command::bug::BugHandler;
use crate::command::cancel::CancelHandler;
use crate::command::checkpoint::CheckpointHandler;
use crate::command::clear::ClearHandler;
use crate::command::cognitive_view::CognitiveViewHandler;
use crate::command::color::ColorHandler;
use crate::command::compact::CompactHandler;
use crate::command::config::ConfigHandler;
use crate::command::context_cmd::ContextHandler;
use crate::command::copy::CopyHandler;
use crate::command::cost::CostHandler;
use crate::command::denials::DenialsHandler;
use crate::command::diff::DiffHandler;
use crate::command::doctor::DoctorHandler;
use crate::command::effort::EffortHandler;
use crate::command::evidence_view::{DocsViewHandler, LearningViewHandler};
use crate::command::export::ExportHandler;
use crate::command::fast::FastHandler;
use crate::command::fork::ForkHandler;
use crate::command::garden::GardenHandler;
use crate::command::help::HelpHandler;
use crate::command::hooks::HooksHandler;
use crate::command::learning_status::LearningStatusHandler;
use crate::command::login::LoginHandler;
use crate::command::logout::LogoutHandler;
use crate::command::mcp::McpHandler;
use crate::command::memory::MemoryHandler;
use crate::command::model::ModelHandler;
use crate::command::permissions::PermissionsHandler;
use crate::command::pipeline_slash::PipelineSlashHandler;
use crate::command::recall::RecallHandler;
use crate::command::release_notes::ReleaseNotesHandler;
use crate::command::reload::ReloadHandler;
use crate::command::rename::RenameHandler;
use crate::command::resume::ResumeHandler;
use crate::command::rules::RulesHandler;
use crate::command::run_agent::RunAgentHandler;
use crate::command::status::StatusHandler;
use crate::command::task::TasksHandler;
use crate::command::theme::ThemeHandler;
use crate::command::thinking::ThinkingHandler;
use crate::command::usage::UsageHandler;
use crate::command::vim::VimHandler;
use crate::command::voice::VoiceHandler;
use crate::command::workflow::WorkflowHandler;

pub(crate) fn default_registry() -> Registry {
    let mut b = RegistryBuilder::new();
    // Primaries FIRST — builder panics on duplicate primary names.
    b.insert_primary("fast", Arc::new(FastHandler));
    b.insert_primary("compact", Arc::new(CompactHandler::new()));
    b.insert_primary("clear", Arc::new(ClearHandler::new()));
    b.insert_primary("export", Arc::new(ExportHandler));
    b.insert_primary("thinking", Arc::new(ThinkingHandler));
    b.insert_primary("effort", Arc::new(EffortHandler));
    b.insert_primary("garden", Arc::new(GardenHandler));
    b.insert_primary("model", Arc::new(ModelHandler));
    b.insert_primary("copy", Arc::new(CopyHandler::new()));
    b.insert_primary("context", Arc::new(ContextHandler));
    b.insert_primary("status", Arc::new(StatusHandler));
    b.insert_primary("cost", Arc::new(CostHandler));
    b.insert_primary("permissions", Arc::new(PermissionsHandler));
    // TASK-TUI-626: /plan Plan Mode toggle (SNAPSHOT+EFFECT via SetPermissionMode("plan")).
    b.insert_primary("plan", Arc::new(crate::command::plan::PlanHandler));
    b.insert_primary("config", Arc::new(ConfigHandler::new()));
    b.insert_primary("memory", Arc::new(MemoryHandler));
    b.insert_primary("doctor", Arc::new(DoctorHandler::new()));
    b.insert_primary("bug", Arc::new(BugHandler));
    b.insert_primary("diff", Arc::new(DiffHandler));
    b.insert_primary("denials", Arc::new(DenialsHandler));
    b.insert_primary("login", Arc::new(LoginHandler::new()));
    b.insert_primary("vim", Arc::new(VimHandler));
    b.insert_primary("usage", Arc::new(UsageHandler::new()));
    b.insert_primary("tasks", Arc::new(TasksHandler));
    // TASK-TUI-623: /tag session tag toggle.
    b.insert_primary("tag", Arc::new(crate::command::tag::TagHandler::new()));
    // TASK-TUI-621: hidden stub — dispatchable when typed explicitly,
    // but OMITTED from archon-tui::commands::all_commands() so the
    // autocomplete / command picker never surfaces it.
    b.insert_primary(
        "teleport",
        Arc::new(crate::command::teleport::TeleportHandler),
    );
    b.insert_primary("release-notes", Arc::new(ReleaseNotesHandler));
    b.insert_primary("reload", Arc::new(ReloadHandler::new()));
    b.insert_primary("logout", Arc::new(LogoutHandler::new()));
    b.insert_primary("help", Arc::new(HelpHandler));
    b.insert_primary("rename", Arc::new(RenameHandler::new()));
    b.insert_primary("resume", Arc::new(ResumeHandler));
    // TASK-TUI-622: /review PR code-review prompt builder.
    b.insert_primary(
        "review",
        Arc::new(crate::command::review::ReviewHandler::new()),
    );
    // TASK-TUI-624: /commit AI git-commit prompt builder.
    b.insert_primary(
        "commit",
        Arc::new(crate::command::commit::CommitHandler::new()),
    );
    // TASK-TUI-628: /sandbox Bubble-mode toggle.
    b.insert_primary(
        "sandbox",
        Arc::new(crate::command::sandbox::SandboxHandler::new()),
    );
    // TASK-TUI-620: /rewind message-selector overlay launcher.
    b.insert_primary(
        "rewind",
        Arc::new(crate::command::rewind::RewindHandler::new()),
    );
    // TASK-TUI-627: /skills skills-menu overlay launcher.
    b.insert_primary(
        "skills",
        Arc::new(crate::command::skills::SkillsHandler::new()),
    );
    // TASK-TUI-625: /session remote-URL + QR code display.
    b.insert_primary(
        "session",
        Arc::new(crate::command::session::SessionHandler::new()),
    );
    b.insert_primary("mcp", Arc::new(McpHandler));
    // TASK-AGS-812: NEW /hooks primary (gap-fix Q4=A, no aliases).
    b.insert_primary("hooks", Arc::new(HooksHandler));
    b.insert_primary("learning-status", Arc::new(LearningStatusHandler));
    b.insert_primary(
        "archon",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::archon()),
    );
    b.insert_primary("docs", Arc::new(DocsViewHandler));
    b.insert_primary("cognitive", Arc::new(CognitiveViewHandler));
    b.insert_primary("learning", Arc::new(LearningViewHandler));
    b.insert_primary(
        "kb",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "kb",
            "Run knowledge-base CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "video",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "video",
            "Manage video evidence",
        )),
    );
    b.insert_primary(
        "prov",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "prov",
            "Run provenance CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "meaning",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "meaning",
            "Run meaning compiler CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "constellation",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "constellation",
            "Run constellation CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "completion",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "completion",
            "Run completion-integrity CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "behaviour",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "behaviour",
            "Run governed-learning behaviour CLI commands from inside the TUI",
        )),
    );
    b.insert_primary("pipeline", Arc::new(PipelineSlashHandler));
    b.insert_primary(
        "auth",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "auth",
            "Run auth CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "chat",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "chat",
            "Run chat CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "reasoning",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "reasoning",
            "Run reasoning-quality CLI commands from inside the TUI",
        )),
    );
    b.insert_primary(
        "briefing",
        Arc::new(crate::command::cli_mirror::CliMirrorHandler::prefixed(
            "briefing",
            "Preview proactive session briefing from inside the TUI",
        )),
    );
    b.insert_primary("fork", Arc::new(ForkHandler));
    b.insert_primary("checkpoint", Arc::new(CheckpointHandler::new()));
    b.insert_primary("add-dir", Arc::new(AddDirHandler));
    b.insert_primary("color", Arc::new(ColorHandler));
    b.insert_primary("theme", Arc::new(ThemeHandler));
    b.insert_primary("recall", Arc::new(RecallHandler::new()));
    b.insert_primary("rules", Arc::new(RulesHandler::new()));
    // TASK-HOTFIX-V0.1.7: /run-agent primary (#248).
    b.insert_primary("run-agent", Arc::new(RunAgentHandler));
    // Deliverable C: /archon-code + /archon-research TUI primaries.
    b.insert_primary("archon-code", Arc::new(ArchonCodeHandler));
    b.insert_primary("archon-research", Arc::new(ArchonResearchHandler));
    b.insert_primary("workflow", Arc::new(WorkflowHandler));
    // TASK-AGS-805: /cancel primary (aliases: stop, abort).
    b.insert_primary("cancel", Arc::new(CancelHandler::new()));
    // TASK-AGS-816: NEW /voice primary (gap-fix Q4=A, no aliases).
    b.insert_primary("voice", Arc::new(VoiceHandler));
    // TASK-#206 SLASH-EXIT: /exit graceful-shutdown handler, alias /q.
    // Alias replaces the dead skill-registry entry previously at
    // src/session.rs:1928 (which pointed to a non-existent `exit` skill).
    b.insert_primary("exit", Arc::new(crate::command::exit::ExitHandler));
    // TASK-#215 SLASH-EXTRA-USAGE: /extra-usage 6-section detailed report.
    // Reuses the existing `usage_snapshot` field; the snapshot population
    // arm in src/command/context.rs is widened to also fire on
    // primary == "extra-usage". No new CommandContext fields.
    b.insert_primary(
        "extra-usage",
        Arc::new(crate::command::extra_usage::ExtraUsageHandler),
    );
    // TASK-#210 SLASH-PROVIDERS: /providers lists 36 registered LLM
    // providers (5 native + 31 OpenAI-compat) by reading the static
    // archon_llm::providers::{list_native, list_compat} registries.
    // GHOST-003: 4 stub providers removed. No CommandContext field needed.
    b.insert_primary(
        "providers",
        Arc::new(crate::command::providers::ProvidersHandler),
    );
    // TASK-#211 SLASH-AGENT: /agent umbrella (list/info/run subcommands).
    // Reads the new agent_registry field on CommandContext (DIRECT
    // pattern; populated unconditionally from SlashCommandContext).
    b.insert_primary("agent", Arc::new(crate::command::agent_slash::AgentHandler));
    // TASK-#212 SLASH-MANAGED-AGENTS: /managed-agents remote-registry
    // status + how-to. Pure status command — no async fetch (deferred
    // to a follow-up; see managed_agents.rs module rustdoc).
    b.insert_primary(
        "managed-agents",
        Arc::new(crate::command::managed_agents::ManagedAgentsHandler),
    );
    // TASK-#213 SLASH-REFRESH: /refresh re-scans the AgentRegistry
    // from disk (sync RwLock::write() + AgentRegistry::reload). Skill
    // refresh is deferred (no Mutex wrapper on the skill registry);
    // WASM plugin hot-reload is deferred to #217.
    b.insert_primary("refresh", Arc::new(crate::command::refresh::RefreshHandler));
    // TASK-#214 SLASH-CONNECT: /connect — list configured MCP servers
    // + emit connect-hint TextDelta. Dynamic in-session connect
    // (wrapping `McpServerManager::enable_server`) is DEFERRED — the
    // upstream `lifecycle::connect_server` future is genuinely !Send
    // (rmcp/tungstenite path), and wrapping it from a `Future + Send +
    // 'a` apply_effect path needs either an upstream Send-cleanup or a
    // session-wide LocalSet — both cross-cutting and out of scope. NO
    // `CommandEffect` variant added; see `src/command/connect.rs`
    // module rustdoc for the full reconciliation.
    b.insert_primary("connect", Arc::new(crate::command::connect::ConnectHandler));
    // TASK-#216 SLASH-PLUGIN: /plugin umbrella — list/info subcommands
    // re-scan disk via the shared `load_plugins_from_default_dirs()`
    // helper; enable/disable/install/reload subcommands emit hint
    // TextDeltas (persistence layer is absent in archon-plugin and
    // adding it is cross-cutting subsystem work — see
    // `src/command/plugin_slash.rs` module rustdoc for the full
    // reconciliation).
    b.insert_primary(
        "plugin",
        Arc::new(crate::command::plugin_slash::PluginSlashHandler),
    );
    // TASK-#217 SLASH-RELOAD-PLUGINS: /reload-plugins disk re-scan via
    // the same shared helper (load_plugins_from_default_dirs). True
    // in-process hot-swap of a running WASM module is DEFERRED —
    // `WasmPluginHost` has no `reload_plugin` API and no session-
    // shared host exists. See `src/command/reload_plugins.rs` module
    // rustdoc for the full reconciliation.
    b.insert_primary(
        "reload-plugins",
        Arc::new(crate::command::reload_plugins::ReloadPluginsHandler),
    );
    // TASK-#207 SLASH-FILES: /files opens a file-picker overlay rooted
    // at working_dir. Walks one level via the screen module's
    // `read_dir_entries` helper (skips dotfiles + common build-
    // artifact dirs); user navigates with Up/Down/Enter/Backspace/Esc;
    // Enter on a file injects `@<absolute-path> ` into the prompt.
    b.insert_primary(
        "files",
        Arc::new(crate::command::files::FilesHandler::new()),
    );
    // TASK-#208 SLASH-SEARCH: /search <query> recursive basename
    // substring match (case-insensitive) over working_dir, capped at
    // 200 results, max_depth 8, same SKIP_DIRS filter as /files.
    // Emits ShowSearchResults overlay; Enter on a result injects
    // `@<absolute-path> ` into the prompt.
    b.insert_primary(
        "search",
        Arc::new(crate::command::search::SearchHandler::new()),
    );
    // TASK-#209 SLASH-SUMMARY: /summary one-glance session headline.
    // TextDelta-only (no overlay) — peer pattern with /usage,
    // /extra-usage, /cost, /status. Reuses the existing
    // `usage_snapshot` field; the snapshot-population arm in
    // src/command/context.rs is widened to fire on
    // primary == "summary" alongside "usage" and "extra-usage".
    b.insert_primary("summary", Arc::new(crate::command::summary::SummaryHandler));
    b.insert_primary(
        "gametheory",
        Arc::new(crate::command::gametheory_slash::GameTheorySlashHandler),
    );
    // Aliases are collected from each handler's aliases() method
    // inside RegistryBuilder::build(). Collisions panic.
    b.build()
}
