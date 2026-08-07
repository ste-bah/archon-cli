//! Turning a session's streamed output into something a browser should read.
//!
//! Split out of `web_runtime.rs` so that file stays under the 500-line ceiling —
//! the gate refuses a file AT 500, not merely over it. These three are the
//! natural seam: none of them touch the session handle or the spawn path, they
//! only shape text on the way out.

use archon_core::env_vars::ArchonEnvVars;
use archon_llm::auth::{AuthProvider, resolve_auth_with_keys};

/// The streamed text if there was any, else whatever the turn returned.
///
/// A turn can stream nothing and still produce a reply — a tool-only turn, or
/// one that errored after the stream closed — so the fallback is not a
/// redundancy.
pub(super) fn finish_reply(streamed: &str, fallback: &str) -> String {
    let streamed = streamed.trim();
    if streamed.is_empty() {
        sanitize_web_reply(fallback)
    } else {
        sanitize_web_reply(streamed)
    }
}

/// Drop the TUI's tool chatter from a reply bound for the browser.
///
/// `[tool] ` lines are progress rendering for a terminal that redraws; a chat
/// bubble keeps them forever. A `done:` line begins a block of tool output that
/// runs until the next blank line, so the skip is stateful rather than
/// per-line.
pub(super) fn sanitize_web_reply(value: &str) -> String {
    let mut lines = Vec::new();
    let mut skipping_tool_output = false;
    for line in value.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[tool] ") {
            skipping_tool_output = trimmed.contains(" done:");
            continue;
        }
        if skipping_tool_output {
            if trimmed.is_empty() {
                skipping_tool_output = false;
            }
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// How this session authenticated, for the startup line.
///
/// Names the mechanism and never the secret.
pub(super) fn auth_label(env_vars: &ArchonEnvVars) -> String {
    match resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    ) {
        Ok(AuthProvider::OAuthToken(_)) => "OAuth".into(),
        Ok(AuthProvider::CodexOAuthToken(_)) => "Codex OAuth".into(),
        Ok(AuthProvider::ApiKey(_)) => "API key".into(),
        Ok(AuthProvider::BearerToken(_)) => "Bearer token".into(),
        Err(_) => "none".into(),
    }
}
