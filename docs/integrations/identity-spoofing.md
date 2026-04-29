# Identity & spoofing

archon-cli can identify itself to the Anthropic API as either Claude Code (`spoof`) or as itself (`native`). Spoofing is on by default and is what lets archon use Claude.ai subscriptions transparently.

## The spoof layers

When `[identity] mode = "spoof"` is set:

1. `x-app: cli` HTTP header
2. `User-Agent: claude-cli/{version} (external, cli)` HTTP header
3. `x-entrypoint: cli` HTTP header
4. Dynamically-discovered `anthropic-beta` headers (probed at first run)
5. `metadata.user_id` field matching Claude Code format
6. `metadata.user_email` (when available from auth)
7. Tool schemas matching Claude Code's tool set
8. System prompt prelude matching Claude Code's default
9. `x-anthropic-billing-header` text block prepended to system prompt
10. `anti_distillation` field (when `anti_distillation = true`)

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
mode = "spoof"                    # "spoof" | "native"
spoof_version = "2.1.89"          # Fallback when no Claude Code install detected
anti_distillation = false         # Inject anti-distillation field
```

## Slash commands

| Command | Purpose |
|---|---|
| `/refresh-identity` | Clear the `anthropic-beta` header cache and re-probe |

## CLI

```bash
archon --identity-spoof          # force spoof mode for this invocation
```

## How beta header probing works

On first startup, archon sends a cheap probe request (Haiku, 1 token) to validate which `anthropic-beta` headers the endpoint accepts. Headers the API rejects are stripped from the cache. The cache lives at `~/.local/share/archon/identity-cache.json` and persists across sessions until invalidated.

Run `/refresh-identity` to clear the cache and re-probe — useful when the endpoint changes (e.g., switching from Anthropic to LiteLLM proxy) or after Anthropic updates their accepted beta surface.

## Native mode

When `[identity] mode = "native"`, archon-cli identifies as itself:

```
User-Agent: archon-cli/{version}
```

No spoofing, no beta header probing, no Claude Code mimicry. Use this when:
- Connecting to an Anthropic-compatible endpoint that doesn't care about Claude Code identity (LiteLLM, Ollama)
- Building/testing archon-cli itself
- The spoofing layer interferes with proxy auth

## Why spoofing exists

The OAuth flow archon-cli uses matches the original Claude Code client (`redirect_uri = http://localhost:{port}/callback`), so existing Claude Code tokens on the same machine work transparently. The spoofing layer extends this — Anthropic's API (and its quotas/billing) treats spoofed requests as Claude Code requests, which is exactly what you want for a Claude.ai subscription.

If your account has API-key billing or you use a proxy, you can switch to `native` mode without losing functionality.

## Auditing what gets sent

```bash
archon --debug api,llm -p "hi"
```

The debug logs include the full request body before send (with API key redacted). Compare against Claude Code's actual request to verify parity.

## See also

- [Authentication setup](../getting-started/installation.md)
- [Configuration](../reference/config.md) — `[identity]` section
