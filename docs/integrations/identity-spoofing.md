# Identity & spoofing

archon-cli can identify itself to the Anthropic API as either Claude Code (`spoof`) or as itself (`native`). Spoofing is on by default and is what lets archon use Claude.ai subscriptions transparently. Codex OAuth is separate: it authenticates the OpenAI Codex provider and does not reuse Anthropic headers.

## Auth options

| Path | Command or setting | Used by |
|---|---|---|
| Anthropic OAuth | `archon auth login --provider anthropic` | Claude-backed TUI sessions, pipelines, subagents |
| Anthropic API key | `ANTHROPIC_API_KEY=sk-ant-api...` | Native Anthropic Messages API calls |
| Anthropic-compatible proxy | Anthropic base URL + compatible key | OpenRouter, DeepSeek, LiteLLM, or similar routes |
| Codex OAuth | `archon auth login --provider openai-codex` | Chat, TUI sessions, tool use, subagents, `/btw`, team runs, and provider-neutral pipelines with `[llm].provider = "openai-codex"` |

Run `archon auth status` to inspect both stored OAuth credentials and the active spoof identity. The command redacts account and client IDs and never prints full tokens.

## The spoof layers

When `[identity] mode = "spoof"` (or auth is OAuth, which forces spoof regardless of config):

1. `x-app: cli` HTTP header (`crates/archon-llm/src/identity.rs:293`)
2. `User-Agent: claude-cli/{version} (external, cli)` HTTP header (`identity.rs:289`)
3. `X-Claude-Code-Session-Id: {session-id}` HTTP header
4. `x-client-request-id: {uuid}` HTTP header (per-request)
5. Dynamically-discovered `anthropic-beta` headers (probed at first run, cached)
6. `metadata.user_id` field matching Claude Code's `{account_uuid|session|fingerprint}` shape (`identity.rs:333-339`)
7. System prompt prelude — `"You are Claude Code, Anthropic's official CLI for Claude."` injected as the first system block when missing
8. `x-anthropic-billing-header: cc_version={version}.{fp}; cc_entrypoint={entrypoint};` text block prepended to the system blocks (`identity.rs:370`)
9. `anti_distillation` field in the request body (when `[identity] anti_distillation = true`; default false)

Note: there is no `x-entrypoint` header and no `metadata.user_email` field — `entrypoint` lives only inside the `x-anthropic-billing-header` billing string.

## Version discovery

archon-cli reads the installed Claude Code version from the installed npm package's `package.json` at startup. The discovery walks `bin/claude` → `../package.json` and extracts the `version` field.

If no Claude Code installation is detected, archon falls back to `[identity] spoof_version` from config (default: `2.1.89`).

A startup log line confirms the version source:
```
[INFO archon::session] message=Spoof version resolved version=2.1.119 version_source=package.json
```

Or when falling back:
```
[INFO archon::session] message=Spoof version resolved version=2.1.89 version_source=config
```

## Config

```toml
[identity]
mode = "clean"                    # "clean" (no spoofing) | "spoof" (mimic Claude Code) | "custom" (user-supplied UA + headers)
spoof_version = "2.1.89"          # Fallback when no Claude Code install detected
spoof_entrypoint = "cli"          # Billing-header `cc_entrypoint=` value
anti_distillation = false         # Inject anti-distillation field
```

The default is `mode = "clean"`. OAuth-backed auth (Anthropic OAuth login or any `sk-ant-oat-*` token in `ANTHROPIC_API_KEY`) **automatically forces spoof mode regardless of this config setting** — see `crates/archon-llm/src/identity.rs::resolve_identity_mode`. This is non-negotiable; the API rejects OAuth tokens without Claude Code identity headers.

Codex OAuth provider configuration lives under `[providers.openai-codex]`:

```toml
[providers.openai-codex]
enabled = true

[providers.openai-codex.spoof]
# Defaults ship in the binary and can be refreshed from a manifest.
# Override only when the upstream Codex client identity changes.

[providers.openai-codex.manifest]
ttl_seconds = 21600
```

## Slash commands

| Command | Purpose |
|---|---|
| `/refresh-identity` | Clear the `anthropic-beta` header cache and re-probe |

## CLI

```bash
archon --identity-spoof          # force spoof mode for this invocation
archon auth login --provider anthropic
archon auth login --provider openai-codex
archon auth status
archon chat --provider openai-codex "explain the current task"
```

## How beta header probing works

On first startup, archon sends a cheap probe request (Haiku, 1 token) to validate which `anthropic-beta` headers the endpoint accepts. Headers the API rejects are stripped from the cache. The cache lives at `~/.config/archon/validated_betas.json` and persists across sessions until invalidated.

Run `/refresh-identity` to clear the cache and re-probe — useful when the endpoint changes (e.g., switching from Anthropic to LiteLLM proxy) or after Anthropic updates their accepted beta surface.

## Clean mode

When `[identity] mode = "clean"` (the default for new installs), archon-cli identifies as itself:

```
User-Agent: archon-cli/{version}
x-app: archon
```

No Claude Code mimicry, no beta header probing, no system-block prelude. Use this when:
- Connecting to an Anthropic-compatible endpoint that doesn't care about Claude Code identity (LiteLLM, Ollama, etc.)
- Building/testing archon-cli itself
- The spoofing layer interferes with proxy auth
- You authenticate with a raw `sk-ant-api-*` API key (clean is fine for API keys)

Clean mode does NOT work with OAuth tokens (`sk-ant-oat-*`). The `resolve_identity_mode` helper detects OAuth at client construction and overrides clean → spoof automatically. There is no way to send OAuth credentials in clean mode.

## Custom mode

When `[identity] mode = "custom"`, archon-cli sends operator-defined headers:

```toml
[identity]
mode = "custom"

[identity.custom]
user_agent = "my-org-cli/1.0"
x_app = "internal-tool"

[identity.custom.extra_headers]
"X-Org-Trace-Id" = "abc123"
```

Use for self-hosted proxies that require organization-specific identification. Like clean mode, custom does not work with OAuth tokens.

## Why spoofing exists

The OAuth flow archon-cli uses matches the original Claude Code client (`redirect_uri = http://localhost:{port}/callback`), so existing Claude Code tokens on the same machine work transparently. The spoofing layer extends this — Anthropic's API (and its quotas/billing) treats spoofed requests as Claude Code requests, which is exactly what you want for a Claude.ai subscription.

If your account has API-key billing or you use a proxy, you can switch to `clean` mode without losing functionality. If `ANTHROPIC_API_KEY` is an OAuth-shaped Claude token (`sk-ant-oat-...`), Archon keeps the Claude Code spoof identity on the wire instead of treating it like a raw API key — see `resolve_identity_mode` for the precedence rules.

## Codex OAuth

Codex OAuth credentials are stored in `~/.archon/.credentials.json` under a separate `openaiCodexOauth` entry. This means a machine can be logged in to both Claude and Codex at the same time:

```bash
archon auth login --provider anthropic
archon auth login --provider openai-codex
archon auth status
```

Select the Codex provider explicitly when you want to route through it:

```bash
archon chat --provider openai-codex "write a migration plan"
```

Or make the main interactive TUI session Codex-backed:

```toml
[llm]
provider = "openai-codex"

[api]
default_model = "gpt-5.4"
```

When `[llm].provider = "openai-codex"`, the TUI skips Anthropic auth bootstrap entirely and constructs a Codex provider from the stored `openaiCodexOauth` credentials. `/btw`, subagent turns, team agents, and provider-neutral pipelines use the same active provider, so Codex-backed sessions do not silently create an Anthropic client.

The Codex kill switch is `ARCHON_CODEX_DISABLED=1`; when set, `archon auth status` reports Codex as disabled and provider construction fails closed.

## Auditing what gets sent

```bash
archon --debug api,llm -p "hi"
```

The debug logs include the full request body before send (with API key redacted). Compare against Claude Code's actual request to verify parity.

## See also

- [Authentication setup](../getting-started/installation.md)
- [Configuration](../reference/config.md) — `[identity]` section
