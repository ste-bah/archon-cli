//! The `AgentConfig` workflow subagents run under.

use std::path::Path;

use anyhow::Result;
use archon_core::agent::AgentConfig;
use archon_core::config::ArchonConfig;

/// Build the `AgentConfig` workflow subagents run under.
///
/// Every field here must come from `config`, not from `AgentConfig::default()`.
/// The default is Anthropic-shaped — `model: "claude-sonnet-4-6"` — so a
/// workflow on a Codex provider asked it for a model Codex cannot serve and got
/// that provider's own fallback instead. The session path never had this
/// problem because it calls `active_session_model`, whose whole purpose is
/// stated by its test: `..._uses_configured_codex_default_when_claude_default_would_leak`.
/// Measured before the fix: 698 of 704 subagent requests ran on the fallback
/// while `[models.openai-codex]` said otherwise, and editing that config
/// changed nothing because it was never read on this path.
///
/// `max_tokens`/`thinking_budget` and the permission rules were silently
/// defaulted for the same reason: a struct-update from `default()` looks
/// complete at the call site while quietly supplying values the operator never
/// chose. `install_workflow_cli_subagent_executor` extends `permission_rules`
/// with project MCP grants, so seeding it from config here is additive.
///
/// `sandbox` and `fs` were defaulted for the same reason and cost more: the
/// workflow CLI never goes through session boot, so `sandbox.backend =
/// "docker"` produced a run whose stages executed on the host while the config
/// said they were isolated (#201 Phase 4). A filesystem that cannot be built
/// fails the call exactly as it fails session boot — degrading quietly to the
/// host is the failure, not the mitigation.
pub(crate) fn workflow_cli_agent_config(
    config: &ArchonConfig,
    cwd: &Path,
    session_id: &str,
) -> Result<AgentConfig> {
    Ok(AgentConfig {
        sandbox: crate::runtime::sandbox_world::isolation_backend(&config.sandbox),
        fs: archon_core::sandbox::sandbox_filesystem(&config.sandbox, cwd)
            .map_err(|error| anyhow::anyhow!("sandbox filesystem unavailable: {error}"))?,
        model: crate::session::active_session_model(config),
        max_tokens: config.api.resolved_max_tokens(),
        thinking_budget: config.api.thinking_budget,
        permission_rules: archon_permissions::rules::RuleSet {
            always_allow: config.permissions.always_allow.clone(),
            always_deny: config.permissions.always_deny.clone(),
            always_ask: config.permissions.always_ask.clone(),
        },
        // Same reason as `sandbox` and `fs`, and the same shape of defect: left
        // to `AgentConfig::default()` this is `Block`, so `read_before_edit`
        // set to `warn` or `off` reached every path except a workflow stage.
        // It failed closed rather than open, which makes it milder, not
        // correct — a knob that silently does not apply is not a knob.
        filesystem: config.filesystem,
        // The same defect once more, and this one does not fail closed.
        // `AgentConfig::default()` is `"auto"`, so `permissions.mode` reached
        // every path except this one: a workflow ran in `auto` however the
        // config or a permission preset was set. A preset writes one permission
        // mode and four sandbox knobs — the four arrived here and the mode did
        // not, so `read-only` gave a workflow no sandbox AND no plan mode,
        // neither half of what was chosen.
        permission_mode: std::sync::Arc::new(tokio::sync::Mutex::new(
            config.permissions.mode.clone(),
        )),
        // Every tool-lifecycle emitter is guarded on this being present, so
        // `None` did not degrade the signal — it removed it. A workflow run
        // produced no ToolStarted/ToolCompleted events and no Bash heartbeat at
        // all: the 30-second line carrying pid, elapsed time and output size,
        // which is the one thing that distinguishes an agent still working from
        // an agent stuck, was absent from the only path that runs unattended
        // for hours. A six-hour stall was invisible for exactly this reason.
        activity_sink: crate::session::session_activity_sink(session_id),
        // Read off the AgentConfig by the interactive path only. The workflow's
        // own ToolContext is a separate literal below, and the guard reads it
        // from there, so this line alone does not deliver it — see the
        // `repeat_tool` assignment in `workflow_tool_context`.
        repeat_tool: config.guard.repeat_tool.clone(),
        working_dir: cwd.to_path_buf(),
        session_id: session_id.to_string(),
        max_tool_concurrency: config.tools.max_concurrency as usize,
        max_subagent_concurrency: config.subagent.max_concurrent.max(1),
        subagent_auto_isolation: config.subagent.auto_isolation,
        subagent_isolation_max_tier: config.subagent.isolation_max_tier,
        // The same defect as the fields above, and the one that decides how
        // long a reasoning model may think: extended thinking sends nothing on
        // the wire, so this guard — not the provider — ends the round. Left to
        // `AgentConfig::default()` it stayed at 600s while the config said
        // otherwise, and every long think restarted the tool loop from its
        // first message.
        subagent_stream_idle_timeout_secs: config.subagent.stream_idle_timeout_secs,
        context: config.context.clone(),
        ..AgentConfig::default()
    })
}
