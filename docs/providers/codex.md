# Codex Provider

Archon's `openai-codex` provider uses ChatGPT/Codex OAuth credentials and keeps
its runtime strategy explicit. It is not the template for every provider:
Anthropic, OpenAI-compatible, cloud, and local providers keep their own runtime
contracts.

## Runtime Modes

```toml
[providers.openai-codex]
runtime = "direct" # direct | auto | app_server
direct_fallback = false
app_server_transport = "websocket" # websocket | stdio
app_server_url = "ws://127.0.0.1:11434/codex"
app_server_command = "codex"
app_server_args = ["app-server"]
app_server_discovery_timeout_ms = 2500
app_server_model_catalog = ["gpt-5.5", "gpt-5.4"]
```

`direct` is the compatibility path and talks through Archon's native Codex
provider. `app_server` talks JSON-RPC over the configured app-server
transport. `auto` selects app-server when the transport target is configured;
otherwise it may use direct only when `direct_fallback = true`. In `auto` mode,
tool-bearing requests also route to the direct Codex runtime when
`direct_fallback = true` so Archon's existing permission preflight, sandbox
backend, hooks, and tool-result loop remain authoritative. Fallbacks are
persisted as redacted provider runtime events in Cozo.

Codex app-server is a JSON-RPC transport. `app_server_transport = "websocket"`
uses `app_server_url`; `app_server_transport = "stdio"` spawns
`app_server_command` with `app_server_args` and talks over stdin/stdout.
`ARCHON_CODEX_APP_SERVER_URL` overrides `app_server_url` for diagnostics. URL
schemes `ws`, `wss`, `http`, and `https` are accepted for compatibility;
invalid targets fail closed and are redacted before persistence.

## Model Defaults

When `[llm].provider = "openai-codex"`, Archon normalizes inherited Claude-shaped
defaults into the configured Codex model aliases before interactive sessions and
subagents see them. This prevents a Codex session from leaking a literal
`claude-*` model id into provider calls. The mapping is controlled by
`[models.openai-codex]`: `default` is used for Sonnet/Opus-tier inherited
defaults, and `mini` is used for Haiku-tier inherited defaults. A concrete
Codex model configured as `api.default_model` is preserved.

## OAuth

Run:

```bash
archon auth login --provider openai-codex
archon auth status
archon providers status --provider openai-codex
archon providers status --provider openai-codex --json
```

Archon stores Codex OAuth separately from Anthropic credentials. If Archon's
credential entry is absent, it can read the official Codex CLI auth file as a
read-only fallback. Codex OAuth never enables Anthropic Claude Code spoofing.

## Rate Limits

Recent Codex rate or usage-limit windows are captured in Cozo and shown with:

```bash
archon providers limits --provider openai-codex
archon providers report --provider openai-codex --json
```

## Safety Notes

Codex app-server fallback is policy controlled and auditable. Forced
`app_server` mode rejects Archon tool schemas rather than allowing provider-side
callbacks to bypass Archon's permission preflight, sandbox backend, hooks, or
Anthropic spoofing invariants. Use `runtime = "auto"` with
`direct_fallback = true` when a session needs both app-server turns and
Archon-managed tools.
