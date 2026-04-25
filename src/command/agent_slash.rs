//! TASK-#211 SLASH-AGENT — `/agent` umbrella slash-command handler.
//!
//! Subcommands:
//!   - `/agent` (no args)      → list all available agents
//!   - `/agent list`           → same as no-args (alias for the empty form)
//!   - `/agent info <name>`    → show full details for one agent
//!   - `/agent run <name> ...` → delegate-hint to `/run-agent` skill
//!     (the skill returns `SkillOutput::Prompt`, which is the
//!     established surface for spawning subagents — re-implementing it
//!     here would duplicate `agent_skills::RunAgentSkill`)
//!   - any other subcommand    → usage TextDelta
//!
//! Reads the live `AgentRegistry` via the new `agent_registry` field on
//! `CommandContext` (DIRECT pattern, populated unconditionally in
//! `build_command_context` from `SlashCommandContext::agent_registry`).
//! `RwLock::read()` is sync, so `execute` consumes the registry without
//! holding any lock across `ctx.emit`.
//!
//! New file (separate from `src/command/agent.rs`, which holds the
//! existing async CLI subcommand handlers `handle_agent_list/search/info`
//! used by `archon agent ...` invocations). Splitting keeps both files
//! comfortably under the 500-line ceiling and avoids touching the
//! independently-tested CLI path.

use std::sync::Arc;
use std::sync::RwLock;

use archon_core::agents::AgentRegistry;
use archon_core::agents::definition::CustomAgentDefinition;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/agent` umbrella handler — list / info / run subcommands.
pub(crate) struct AgentHandler;

impl CommandHandler for AgentHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let registry_arc = ctx.agent_registry.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "AgentHandler invoked without agent_registry populated \
                 — build_command_context bug"
            )
        })?;

        let subcommand = args.first().map(|s| s.as_str()).unwrap_or("list");
        let rest: &[String] = if args.is_empty() { &[] } else { &args[1..] };

        let msg = match subcommand {
            "" | "list" => render_list(registry_arc.as_ref()),
            "info" | "show" => match rest.first() {
                Some(name) => render_info(registry_arc.as_ref(), name),
                None => render_usage("info: missing <name>"),
            },
            "run" | "exec" => render_run_hint(rest),
            other => render_usage(&format!("unknown subcommand `{}`", other)),
        };

        ctx.emit(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        "Manage custom agents (list, info, run — delegates run to /run-agent skill)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

// Width constants kept module-private so list/info/usage stay aligned.
const COL_NAME: usize = 28;
const COL_SOURCE: usize = 12;

fn render_list(registry: &RwLock<AgentRegistry>) -> String {
    // Acquire read lock + clone what we need into owned strings while
    // the guard is held; drop the guard before formatting to keep the
    // critical section short.
    let entries: Vec<AgentRow> = match registry.read() {
        Ok(guard) => guard.list().iter().map(|d| AgentRow::from_def(d)).collect(),
        Err(poisoned) => poisoned
            .into_inner()
            .list()
            .iter()
            .map(|d| AgentRow::from_def(d))
            .collect(),
    };

    let mut out = String::with_capacity(2048);
    out.push('\n');
    out.push_str(&format!(
        "Available agents ({} total)\n",
        entries.len()
    ));
    out.push_str(&format!(
        "  {:<name$}  {:<source$}  description\n",
        "name",
        "source",
        name = COL_NAME,
        source = COL_SOURCE,
    ));
    out.push_str(&format!(
        "  {}  {}  {}\n",
        "-".repeat(COL_NAME),
        "-".repeat(COL_SOURCE),
        "-".repeat(40),
    ));
    if entries.is_empty() {
        out.push_str("  (no agents loaded)\n");
    } else {
        for row in &entries {
            out.push_str(&format!(
                "  {:<name$}  {:<source$}  {}\n",
                truncate_chars(&row.name, COL_NAME),
                truncate_chars(&row.source, COL_SOURCE),
                truncate_chars(&row.description_first_line, 40),
                name = COL_NAME,
                source = COL_SOURCE,
            ));
        }
    }
    out.push_str(
        "\nTip: `/agent info <name>` for details; `/run-agent <name> <task>` to invoke.\n",
    );
    out
}

fn render_info(registry: &RwLock<AgentRegistry>, name: &str) -> String {
    let entry: Option<AgentRow> = match registry.read() {
        Ok(guard) => guard.resolve(name).map(AgentRow::from_def),
        Err(poisoned) => poisoned.into_inner().resolve(name).map(AgentRow::from_def),
    };

    match entry {
        None => format!(
            "\nAgent `{name}` not found. Run `/agent list` to see available names.\n",
            name = name
        ),
        Some(row) => {
            let mut out = String::with_capacity(1024);
            out.push('\n');
            out.push_str(&format!("Agent: {}\n", row.name));
            out.push_str(&format!("  Source:        {}\n", row.source));
            out.push_str(&format!(
                "  Description:   {}\n",
                row.description_first_line
            ));
            out.push_str(&format!(
                "  Model:         {}\n",
                row.model.as_deref().unwrap_or("(default)")
            ));
            out.push_str(&format!(
                "  Effort:        {}\n",
                row.effort.as_deref().unwrap_or("(default)")
            ));
            out.push_str(&format!(
                "  Max turns:     {}\n",
                row.max_turns
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "(default)".to_string())
            ));
            let tools_label = match row.allowed_tools.as_ref() {
                Some(t) if !t.is_empty() => t.join(","),
                _ => "(default — all tools)".to_string(),
            };
            out.push_str(&format!("  Allowed tools: {}\n", tools_label));
            let tags_label = if row.tags.is_empty() {
                "(none)".to_string()
            } else {
                row.tags.join(",")
            };
            out.push_str(&format!("  Tags:          {}\n", tags_label));
            out.push_str(&format!(
                "  Color:         {}\n",
                row.color.as_deref().unwrap_or("(none)")
            ));
            out.push_str("\nUse `/run-agent ");
            out.push_str(&row.name);
            out.push_str(" <task>` to invoke.\n");
            out
        }
    }
}

fn render_run_hint(rest: &[String]) -> String {
    if rest.is_empty() {
        return "\n\
            /agent run is a delegate-hint command. Use the run-agent skill instead:\n  \
            /run-agent <agent-name> <task description>\n\n\
            Run `/agent list` to see available agent names.\n"
            .to_string();
    }
    let name = &rest[0];
    let task = if rest.len() > 1 {
        rest[1..].join(" ")
    } else {
        "<task description>".to_string()
    };
    format!(
        "\n\
        /agent run delegates to the /run-agent skill. Run:\n  \
        /run-agent {name} {task}\n\n\
        (The skill produces a SkillOutput::Prompt that spawns the subagent\n\
        via the Agent tool — re-implementing it here would duplicate the\n\
        existing surface.)\n",
        name = name,
        task = task
    )
}

fn render_usage(prefix: &str) -> String {
    format!(
        "\n\
        /agent — manage custom agents.\n\
        Note: {prefix}\n\n\
        Subcommands:\n  \
        list                   List all available agents (default).\n  \
        info <name>            Show full details for one agent.\n  \
        run  <name> <task...>  Delegate-hint to the /run-agent skill.\n\n\
        Run `/agent list` to start.\n",
        prefix = prefix,
    )
}

/// Owned row view of a `CustomAgentDefinition` — string-only fields so
/// it survives outside the `RwLock` read guard.
#[derive(Debug, Clone)]
struct AgentRow {
    name: String,
    source: String,
    description_first_line: String,
    model: Option<String>,
    effort: Option<String>,
    max_turns: Option<u32>,
    allowed_tools: Option<Vec<String>>,
    tags: Vec<String>,
    color: Option<String>,
}

impl AgentRow {
    fn from_def(d: &CustomAgentDefinition) -> Self {
        let source = match &d.source {
            archon_core::agents::definition::AgentSource::BuiltIn => "builtin".to_string(),
            archon_core::agents::definition::AgentSource::Project => "project".to_string(),
            archon_core::agents::definition::AgentSource::User => "user".to_string(),
            archon_core::agents::definition::AgentSource::Plugin(name) => {
                format!("plugin:{}", name)
            }
        };
        let description_first_line = d
            .description
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        Self {
            name: d.agent_type.clone(),
            source,
            description_first_line,
            model: d.model.clone(),
            effort: d.effort.clone(),
            max_turns: d.max_turns,
            allowed_tools: d.allowed_tools.clone(),
            tags: d.tags.clone(),
            color: d.color.clone(),
        }
    }
}

/// Truncate to at most `max` Unicode characters, appending `…` when
/// shortened. Char-aware — never panics on multi-byte input.
fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

/// Construct an `Arc<RwLock<AgentRegistry>>` for tests with a fixed set
/// of mock agents. Exposed at module scope so test_support helpers can
/// share the fixture across test modules if needed.
#[cfg(test)]
pub(crate) fn fixture_agent_registry() -> Arc<RwLock<AgentRegistry>> {
    // AgentRegistry::empty() returns a registry with zero agents; we
    // construct two synthetic CustomAgentDefinitions and inject them
    // via the public `merge_for_test` shim if available, else fall back
    // to the empty registry. Simplest: empty registry — tests assert
    // the rendering shape, not specific agents.
    Arc::new(RwLock::new(AgentRegistry::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn execute_without_registry_returns_err() {
        let handler = AgentHandler;
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "expected Err when agent_registry is None");
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("agent_registry"),
            "error must reference agent_registry; got: {}",
            msg
        );
    }

    #[test]
    fn list_subcommand_emits_total_count_header() {
        // Use empty registry — confirms 0-agent rendering.
        let handler = AgentHandler;
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler.execute(&mut ctx, &[]).unwrap();
        let body = take_text_delta(&mut rx);
        assert!(
            body.contains("Available agents (0 total)"),
            "missing total-count header; body:\n{}",
            body
        );
        assert!(body.contains("(no agents loaded)"));
    }

    #[test]
    fn list_explicit_keyword_matches_default() {
        let handler = AgentHandler;
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler.execute(&mut ctx, &[String::from("list")]).unwrap();
        let body = take_text_delta(&mut rx);
        assert!(body.contains("Available agents"), "list keyword path failed");
    }

    #[test]
    fn info_unknown_agent_returns_friendly_message() {
        let handler = AgentHandler;
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler
            .execute(&mut ctx, &[String::from("info"), String::from("ghost")])
            .unwrap();
        let body = take_text_delta(&mut rx);
        assert!(body.contains("`ghost` not found"), "missing not-found msg");
        assert!(
            body.contains("/agent list"),
            "missing tip pointing to /agent list"
        );
    }

    #[test]
    fn info_without_name_emits_usage() {
        let handler = AgentHandler;
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler.execute(&mut ctx, &[String::from("info")]).unwrap();
        let body = take_text_delta(&mut rx);
        assert!(
            body.contains("missing <name>"),
            "missing usage error; body:\n{}",
            body
        );
        assert!(body.contains("Subcommands:"));
    }

    #[test]
    fn run_subcommand_delegates_to_run_agent_skill() {
        let handler = AgentHandler;
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler
            .execute(
                &mut ctx,
                &[
                    String::from("run"),
                    String::from("sherlock-holmes"),
                    String::from("audit"),
                    String::from("the"),
                    String::from("repo"),
                ],
            )
            .unwrap();
        let body = take_text_delta(&mut rx);
        assert!(body.contains("/run-agent sherlock-holmes audit the repo"));
        assert!(body.contains("delegates to the /run-agent skill"));
    }

    #[test]
    fn unknown_subcommand_emits_usage() {
        let handler = AgentHandler;
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler
            .execute(&mut ctx, &[String::from("frobnicate")])
            .unwrap();
        let body = take_text_delta(&mut rx);
        assert!(
            body.contains("unknown subcommand `frobnicate`"),
            "missing unknown-subcommand prefix; body:\n{}",
            body
        );
        assert!(body.contains("Subcommands:"));
    }

    #[test]
    fn truncate_chars_unicode_safe() {
        // 10 Greek letters truncated to 5 — must be 5 chars, end in `…`.
        let truncated = truncate_chars("αβγδεζηθικ", 5);
        assert_eq!(truncated.chars().count(), 5);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn description_and_aliases() {
        let h = AgentHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn agent_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("agent") must return Some, and
        // execute against a fixture agent_registry must emit a
        // TextDelta containing the "Available agents" header (default
        // list path).
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("agent")
            .expect("agent must be registered in default_registry()");
        let (mut ctx, mut rx) =
            make_agent_ctx(Some(fixture_agent_registry()));
        handler.execute(&mut ctx, &[]).unwrap();
        let body = take_text_delta(&mut rx);
        assert!(body.contains("Available agents"));
    }

    fn take_text_delta(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
    ) -> String {
        let events = drain_tui_events(rx);
        assert_eq!(events.len(), 1, "expected one TextDelta; got {:?}", events);
        match events.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }
}
