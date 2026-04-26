//! TASK-#212 SLASH-MANAGED-AGENTS — `/managed-agents` slash-command handler.
//!
//! Surfaces a help-and-status TextDelta describing the *managed* (remote)
//! agent registry path:
//!
//!   - explains what a managed agent is (remote registry entry, vs the
//!     locally-loaded `AgentRegistry` surface that backs `/agent`)
//!   - reports whether `ARCHON_REGISTRY_URL` is set in the current
//!     environment (and what its value is)
//!   - points the user at the canonical CLI surface
//!     (`archon agent search --registry-url <URL>`) which DOES actually
//!     hit the remote registry and render the listing
//!
//! # Spec/reality reconciliation
//!
//! The mission ticket said "remote registry lister". Reality on this
//! branch:
//!
//!   1. `archon-core::agents::discovery::remote::RemoteDiscoverySource::
//!      load_all` is async (HTTP via `reqwest`), but
//!      `CommandHandler::execute` is sync (Q1=A invariant). To actually
//!      fetch from a sync handler the work must move into
//!      `build_command_context` (which IS async) via a new SNAPSHOT
//!      field on `CommandContext`.
//!   2. There is no session-shared registry URL config. The CLI subcommand
//!      `archon agent search --registry-url <URL>` accepts the URL per
//!      invocation; nothing carries it across to the slash dispatch
//!      surface. There is no default URL constant, no env var
//!      consumed by the existing remote-source constructor.
//!   3. Adding a session-shared `Arc<RemoteRegistryConfig>` field on
//!      `SlashCommandContext` + a `managed_agents_snapshot` field on
//!      `CommandContext` + an async builder would cross the
//!      wrapper-scope ceiling for this ticket — it is a small subsystem,
//!      not a wrapper.
//!
//! Resolution per the mission "Spec-reality drift → reconcile, adapt
//! scope, document in commit body" rule:
//!
//!   - Ship `/managed-agents` as a STATUS + HOW-TO command. It tells
//!     the user (a) what the surface is for, (b) whether
//!     `ARCHON_REGISTRY_URL` is set and what its value is, (c) the
//!     exact CLI invocation that DOES fetch. No async work in the
//!     handler, no new context fields, no flaky network in the
//!     dispatch path.
//!   - Defer the actual remote fetch + render to a follow-up ticket
//!     (see commit body for the full TODO list — needs a snapshot
//!     pattern with `RemoteDiscoverySource::load_all` in the builder
//!     + a session-shared validator + UI flow for stale-cache
//!     fallback).
//!
//! Reading the env var `std::env::var("ARCHON_REGISTRY_URL")` at
//! handler dispatch time is safe-by-design here: the rendered output
//! is a help/status string, not a cached fetch result, so no
//! test-isolation hazard and no race against env mutations.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/managed-agents` handler — emits a status + how-to TextDelta.
pub(crate) struct ManagedAgentsHandler;

impl CommandHandler for ManagedAgentsHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let registry_url = std::env::var("ARCHON_REGISTRY_URL").ok();
        let body = render_status(registry_url.as_deref());
        ctx.emit(TuiEvent::TextDelta(body));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show managed-agent (remote-registry) status + how to fetch the listing"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Render the status + how-to body. Pulled out so unit tests can
/// exercise both the env-set and env-unset branches without poking
/// `std::env::set_var` (which would cross-contaminate concurrent
/// tests).
fn render_status(registry_url: Option<&str>) -> String {
    let mut out = String::with_capacity(1024);
    out.push('\n');
    out.push_str("Managed agents (remote registry) — status\n");
    out.push_str("─────────────────────────────────────────\n");
    out.push_str(
        "A *managed* agent is a record discovered from a remote\n\
         agent-registry HTTP endpoint, validated against the shared\n\
         schema, and surfaced for browse / install. This is distinct\n\
         from the LOCAL agent surface (`/agent list`) which lists\n\
         agents already loaded into the in-process AgentRegistry from\n\
         .archon/agents/, ~/.archon/agents/, and plugin directories.\n\n",
    );

    out.push_str("Configuration\n");
    match registry_url {
        Some(url) if !url.trim().is_empty() => {
            out.push_str(&format!("  ARCHON_REGISTRY_URL: {url}\n"));
            out.push_str("  Status: configured\n");
        }
        _ => {
            out.push_str("  ARCHON_REGISTRY_URL: (unset)\n");
            out.push_str("  Status: not configured\n");
        }
    }

    out.push('\n');
    out.push_str("To fetch and list the remote registry, use the CLI\n");
    out.push_str("subcommand (which threads the URL through to the\n");
    out.push_str("async RemoteDiscoverySource::load_all path):\n\n");
    out.push_str("  archon agent search --registry-url <URL>\n");
    out.push_str("  archon agent search --registry-url <URL> --tag <tag>\n");
    out.push_str("  archon agent search --registry-url <URL> --capability <cap>\n\n");

    out.push_str("Notes\n");
    out.push_str(
        "  • The slash-command surface (`/managed-agents`) is a status +\n  \
           how-to today; live remote fetching is deferred to a follow-up\n  \
           ticket that adds a SNAPSHOT field + async builder. Network\n  \
           work cannot run from a sync CommandHandler::execute, so a\n  \
           wrapper-scope ticket cannot ship it without crossing into\n  \
           subsystem refactor territory.\n",
    );
    out.push_str("  • Use `/agent list` for agents already loaded in this session.\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    fn render() -> String {
        let handler = ManagedAgentsHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match events.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn render_status_unset_branch() {
        let body = render_status(None);
        assert!(body.contains("ARCHON_REGISTRY_URL: (unset)"));
        assert!(body.contains("Status: not configured"));
        assert!(body.contains("archon agent search --registry-url"));
        assert!(body.contains("Managed agents (remote registry) — status"));
    }

    #[test]
    fn render_status_set_branch() {
        let body = render_status(Some("https://registry.example.com/agents.json"));
        assert!(body.contains("ARCHON_REGISTRY_URL: https://registry.example.com/agents.json"));
        assert!(body.contains("Status: configured"));
        assert!(!body.contains("(unset)"));
    }

    #[test]
    fn render_status_empty_string_treated_as_unset() {
        // Empty string env var should not be treated as a real URL.
        let body = render_status(Some(""));
        assert!(body.contains("(unset)"));
        assert!(body.contains("Status: not configured"));
    }

    #[test]
    fn render_status_whitespace_only_treated_as_unset() {
        let body = render_status(Some("   "));
        assert!(body.contains("(unset)"));
        assert!(body.contains("Status: not configured"));
    }

    #[test]
    fn execute_emits_single_text_delta() {
        let body = render();
        // Don't assert on env-set vs env-unset path here — the test
        // process may or may not have ARCHON_REGISTRY_URL set. Just
        // verify that ONE of the expected configuration lines is
        // present (mutually exclusive in the renderer).
        let configured = body.contains("Status: configured");
        let unconfigured = body.contains("Status: not configured");
        assert!(
            configured ^ unconfigured,
            "exactly one of configured/not-configured must appear; \
             got configured={} unconfigured={}; body:\n{}",
            configured,
            unconfigured,
            body
        );
    }

    #[test]
    fn body_explains_local_vs_managed_distinction() {
        let body = render_status(None);
        // Cross-reference to the LOCAL surface so users can find it.
        assert!(body.contains("/agent list"));
        assert!(body.contains("LOCAL"));
    }

    #[test]
    fn description_and_aliases() {
        let h = ManagedAgentsHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn managed_agents_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("managed-agents") must return Some,
        // and execute against a bug-ctx must emit a single TextDelta
        // containing the canonical header and the CLI hint line.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("managed-agents")
            .expect("managed-agents must be registered in default_registry()");
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        let body = match &events[0] {
            TuiEvent::TextDelta(s) => s.clone(),
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("Managed agents (remote registry)"));
        assert!(body.contains("archon agent search --registry-url"));
    }
}
