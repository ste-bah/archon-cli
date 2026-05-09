# Anthropic Claude Code

Anthropic OAuth is the strictest provider compatibility surface in Archon. When
Anthropic OAuth or an OAuth-shaped `sk-ant-oat-*` token is used, Archon must send
Claude Code-compatible identity data.

## Spoof Contract

The protected spoof contract includes:

- `x-app: cli`
- Claude CLI-shaped `User-Agent`
- `X-Claude-Code-Session-Id`
- `x-client-request-id`
- accepted `anthropic-beta` headers
- `anthropic-version: 2023-06-01`
- JSON-string `metadata.user_id`
- Claude Code system prelude and billing-header-shaped system block

API-key Anthropic auth can use clean or custom identity. OAuth cannot be sent in
clean mode; Archon forces spoof mode for OAuth because the provider expects
Claude Code-compatible requests.

## Provider Runtime

Anthropic runtime status is provider-neutral: construction, fallback, auth
profile selection, status snapshots, and provider failures are recorded in Cozo
where durable. Temporary caches such as beta-header probes may remain cache
files.

Use:

```bash
archon auth status
archon providers status --provider anthropic
archon providers profiles select anthropic --auth-kind oauth
```

## What Must Not Change

Do not move Anthropic spoof headers into sandbox backends, OpenShell provider
injection, or Codex app-server strategy. Spoof identity is host-side provider
runtime behavior and never implies permission bypass.
