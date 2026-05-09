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
app_server_url = "http://127.0.0.1:11434/codex"
app_server_discovery_timeout_ms = 2500
app_server_model_catalog = ["gpt-5.5", "gpt-5.4"]
```

`direct` is the compatibility path and talks through Archon's native Codex
provider. `app_server` currently fails closed because the app-server transport
adapter is not implemented. `auto` may use direct only when
`direct_fallback = true`; the fallback is persisted as a redacted provider
runtime event in Cozo.

`ARCHON_CODEX_APP_SERVER_URL` overrides `app_server_url` for diagnostics. The
endpoint must be `http` or `https`; invalid values fail closed and are redacted
before persistence.

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

Codex app-server fallback is policy controlled and auditable. It must not
silently bypass the provider runtime event store, permission preflight, sandbox
backend, or Anthropic spoofing invariants.
