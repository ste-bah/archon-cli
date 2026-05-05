# Codex authentication

Setup guide for users with a ChatGPT subscription who want to route some of their archon-cli usage through OpenAI's Codex provider instead of (or alongside) Anthropic.

> **TUI parity.** Every shell command in this doc has a slash equivalent inside the TUI: `archon auth login` ↔ `/auth login`, `archon auth status` ↔ `/auth status`, `archon chat` ↔ `/chat`. See [CLI and TUI Command Parity](../cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity).

## What you get with Codex

| Surface | Codex support | Notes |
|---|---|---|
| `/chat --provider openai-codex` | ✅ supported | Single-turn prompt, streaming or `--no-stream` response |
| `archon chat --provider openai-codex` | ✅ supported | Same as the slash form, run from a shell |
| `archon` (interactive TUI session) | ✅ supported when `[llm].provider = "openai-codex"` | The main conversational session uses Codex OAuth instead of Anthropic OAuth |
| `/archon-code` and `/archon-research` | ✅ supported when `[llm].provider = "openai-codex"` | Provider-neutral pipeline adapter routes through the active provider |
| `/gametheory` | ✅ supported when `[llm].provider = "openai-codex"` | Classification and specialist calls use the active provider |
| `/run-agent`, `Agent` tool, subagents | ✅ supported when `[llm].provider = "openai-codex"` | Provider-neutral subagent runner uses the active provider and Codex tool-result continuation |
| `archon team run` | ✅ supported when `[llm].provider = "openai-codex"` | Team execution builds the configured provider through the shared runtime router |
| `archon completion ...` | ✅ provider-neutral | Completion integrity currently verifies persisted evidence/trust state rather than calling a provider |
| `archon auth login/status/logout --provider openai-codex` | ✅ supported | Manage the credential lifecycle |

In short: **Codex can back chat, the main interactive TUI, tool use, subagents, `/btw`, provider-neutral pipelines, and team runs.** Exact cost metadata still stays honest: if the backend does not expose pricing/usage, Archon marks cost metadata unsupported instead of inventing it. You can keep both Anthropic and Codex credentials on the same machine and choose the provider per surface.

For the generated source-of-truth matrix, run `archon providers capabilities` or `/providers capabilities`, or read [Provider capabilities](../generated/provider-capabilities.md).

## Prerequisites

- An active **ChatGPT** subscription (Plus, Pro, Business, Team, or Enterprise — any tier with API access via the Codex client).
- archon-cli v0.1.40 or newer.
- A browser on the same machine that will run `archon auth login` (the OAuth callback round-trips through `localhost`).

## Read this BEFORE you log in

archon-cli surfaces the following warning before the first Codex login. Read it. Run `archon auth login --provider openai-codex --accept-tos` only after you have done so.

```
WARNING: Codex authentication via archon-cli

archon-cli authenticates against ChatGPT subscription tokens (auth.openai.com)
and uses an undocumented internal API at chatgpt.com/backend-api/codex.

Risks:
  1. OpenAI may change or restrict this API without notice.
  2. ChatGPT subscription terms may restrict programmatic access.
  3. By default, archon-cli identifies as 'openclaw'. You can override this
     in config.toml.

Legal guardrail:
  - archon-cli REJECTS user-agent strings starting with 'ChatGPT/', 'OpenAI/',
    'ChatGPT-', or 'OpenAI-' to prevent impersonation of OpenAI's own products.
  - You are SOLELY responsible for any other identity choice you configure.

Mitigations:
  - Disable Codex entirely: set ARCHON_CODEX_DISABLED=1
  - Customize identity: edit [providers.openai-codex.spoof] in config.toml
  - Hot-update spoof config: manifest refreshes every 6 hours by default
```

The warning text lives in `src/command/auth.rs::TOS_WARNING`. archon-cli will not start the OAuth flow on first use unless you either type `y` at the prompt or pass `--accept-tos` explicitly. Acknowledgement is recorded at `~/.archon/codex-tos-ack` and silences the prompt on subsequent logins.

## Login

### Inside the TUI

```
> /auth login --provider openai-codex
Starting Codex OAuth login...
Open this URL to continue Codex login: https://auth.openai.com/oauth/authorize?...
```

Click the URL (the TUI prints it on stderr so click-to-open works in most terminals). Complete the OAuth dance in the browser. archon-cli's local callback receives the code, exchanges it for tokens, and writes them to `~/.archon/.credentials.json` under the `openaiCodexOauth` key.

```
Codex login successful.
```

### From a shell (equivalent)

```bash
archon auth login --provider openai-codex
```

If you want to skip the TOS prompt for scripted use after you've read it once:

```bash
archon auth login --provider openai-codex --accept-tos
```

## Verify the login took

```
> /auth status
```

or

```bash
archon auth status
```

Expected output (assuming both providers are logged in):

```
Anthropic (Claude)
  Status:        authenticated
  Token expires: 2026-06-04 12:34:00 UTC
  Subscription:  pro

Codex (OpenAI ChatGPT subscription)
  Status:           authenticated as account acct_*****d2f1
  Token expires:    2026-05-11 18:22:00 UTC
  Spoof identity:   from bundled manifest
    originator:     openclaw
    user-agent:     openclaw/0.1.40
    client-id:      app_***************
    openai-beta:    responses=experimental
  Manifest:         https://archon-public.s3.amazonaws.com/codex-compat.json
  Kill-switch:      enabled (set ARCHON_CODEX_DISABLED=1 to disable)
```

`account_id` is partially redacted (`acct_*****d2f1`) and the OAuth client ID is also redacted (`app_***************`). archon-cli **never** prints raw tokens.

## Use Codex for a one-shot chat

Once authenticated:

```
> /chat --provider openai-codex "summarize this repository in one paragraph"
```

or

```bash
archon chat --provider openai-codex "summarize this repository in one paragraph"
```

Flags:

| Flag | Default | Notes |
|---|---|---|
| `--provider` | `anthropic` | Set to `openai-codex` to route this turn through Codex. |
| `--model` | `gpt-5.4` (Codex), `claude-sonnet-4-6` (Anthropic) | Override per invocation. The Codex default tracks the current Codex CLI ChatGPT-account model. |
| `--no-stream` | streaming on | Print the full response after completion instead of streaming. |
| `--max-tokens` | `1024` | Maximum output tokens for this single turn. |

The `chat` surface is intentionally minimal: one user prompt → one assistant response, no session history, no tool use, no agents. For richer interaction, drop into the full TUI (`archon`) and choose the backing provider with `[llm].provider`.

## Use Codex for the full TUI session

To make the main interactive TUI use Codex, set the session provider in `config.toml`:

```toml
[llm]
provider = "openai-codex"

[api]
default_model = "gpt-5.4"
```

Then start Archon normally:

```bash
archon
```

In this mode Archon skips the Anthropic auth bootstrap and builds the session agent from the stored `openaiCodexOauth` credentials. If `default_model` is still a Claude-shaped value, Archon automatically uses `gpt-5.4` for the Codex-backed session.

`/btw` side questions use the same active session provider. In a Codex-backed TUI session, `/btw what did you just decide?` is sent through Codex OAuth; in an Anthropic-backed session, the same command is sent through Anthropic OAuth/API key/proxy.

## Logout

Just Codex (preserves Anthropic credentials):

```bash
archon auth logout --provider openai-codex
```

Just Anthropic (preserves Codex):

```bash
archon auth logout --provider anthropic
```

Both at once (deletes the credentials file if no other providers remain):

```bash
archon auth logout
```

The slash form is identical: `/auth logout --provider openai-codex`.

## Kill switch

Disable Codex entirely without removing the credentials:

```bash
export ARCHON_CODEX_DISABLED=1
```

Set this in your shell rc file to make it permanent. With the kill switch on:

- `archon auth status` reports Codex as `DISABLED via ARCHON_CODEX_DISABLED=1`.
- `archon chat --provider openai-codex` fails closed at provider construction.
- The credentials in `~/.archon/.credentials.json` are untouched and reactivate the moment you unset the env var.

Use this if you want to keep your Codex tokens around but pause programmatic use (e.g., during a billing review).

## Spoof identity

By default archon-cli identifies as `openclaw` to OpenAI's Codex backend. The spoof identity is enforced — user-agent strings starting with `ChatGPT/`, `ChatGPT-`, `OpenAI/`, or `OpenAI-` are **rejected at config-load time** to prevent impersonation of OpenAI's own products.

Override the spoof identity in `config.toml`:

```toml
[providers.openai-codex.spoof]
originator = "my-org-tool"
user_agent = "my-org-tool/1.0"
# client_id  = "..."   # OAuth client id; only override if your org has a registered Codex client
# openai_beta = "..."  # OpenAI-Beta header value
```

Or via environment variables (precedence depends on `policy.workers` config):

```bash
export ARCHON_CODEX_ORIGINATOR=my-org-tool
export ARCHON_CODEX_USER_AGENT=my-org-tool/1.0
```

See [docs/env-vars-codex.md](../env-vars-codex.md) for the full env var list.

## Manifest refresh

archon-cli ships with a bundled compatibility manifest (`crates/archon-llm/resources/codex-compat.json`) and refreshes it from the upstream manifest URL every 6 hours by default. Override:

```toml
[providers.openai-codex.manifest]
fetch_url = "https://your-org/codex-compat.json"
ttl_seconds = 21600
cache_dir = "~/.cache/archon/codex-manifest"
```

The manifest carries the latest known-good `originator`, `user-agent`, `client-id`, and `openai-beta` values. When OpenAI changes the Codex client identity, you can update the manifest without shipping a new archon-cli release.

## Troubleshooting

### "Codex login cancelled."

You answered `n` (or anything other than `y`) at the TOS prompt. Re-run with `--accept-tos` if you've read the warning and want to proceed without the interactive prompt.

### "failed to resolve Codex spoof identity"

The provider could not load a spoof identity from any of: env vars, config.toml `[providers.openai-codex.spoof]`, the cached manifest, or the bundled `codex-compat.json`. Check:

1. `ARCHON_CODEX_USER_AGENT` (if set) doesn't start with a forbidden prefix.
2. `~/.cache/archon/codex-manifest/` is writable.
3. The bundled manifest exists in your binary (`strings target/release/archon | grep codex-compat`).

### "Token expires" is in the past

Your Codex OAuth token has expired and the refresh attempt failed. Either:

- Re-login: `archon auth login --provider openai-codex`.
- Or check that `~/.archon/.credentials.json` has a `refresh_token` field under `openaiCodexOauth`. If it doesn't, the original login was incomplete.

### `archon chat --provider openai-codex` returns "rate limited" immediately

Same shape as the Anthropic OAuth bug we fixed in v0.1.39, but for Codex — the OpenAI backend is rejecting the request without proper headers. Check `archon auth status` to confirm the spoof identity loaded successfully and matches what `chat` is sending. If `Spoof identity: unavailable (...)` appears, fix that first.

### `archon auth status` shows the right account but `chat` fails

Run with debug logs:

```bash
RUST_LOG=archon_llm=debug archon chat --provider openai-codex "test"
```

The full request body (with API token redacted) is logged before send. Compare the headers against the `Spoof identity` block in `archon auth status` — they should match.

### "Status: DISABLED via ARCHON_CODEX_DISABLED=1"

You (or your shell rc) set the kill switch. Unset it:

```bash
unset ARCHON_CODEX_DISABLED
```

## Daily smoke / CI

The Codex daily smoke runbook lives at [docs/maintenance/codex-smoke.md](../maintenance/codex-smoke.md). It exercises credential restore, OAuth refresh, `archon auth status`, and `archon chat --provider openai-codex` against a dedicated ChatGPT Plus test account. Use it as the reference for how to validate Codex auth in your own CI.

## Where credentials live

```
~/.archon/.credentials.json
```

with this top-level shape:

```json
{
  "claudeAiOauth":     { "accessToken": "...", "refreshToken": "...", "expiresAt": 0, ... },
  "openaiCodexOauth":  { "accessToken": "...", "refreshToken": "...", "expiresAt": 0, "accountId": "..." }
}
```

The file is created with mode `0600` and atomic-replaced on every refresh. Both providers can be present simultaneously (one machine logged into both), or just one. `archon auth logout --provider X` removes only the named provider's key; `archon auth logout` (no flag) removes both. If the file ends up empty, it's deleted.

For the deeper mechanics of how OAuth credentials become wire headers, see [identity-spoofing.md](../integrations/identity-spoofing.md).

## See also

- [Identity & spoofing](../integrations/identity-spoofing.md) — the spoof-mode mechanics for both providers
- [Codex environment variables](../env-vars-codex.md) — full `ARCHON_CODEX_*` env var reference
- [Codex daily smoke runbook](../maintenance/codex-smoke.md) — CI/operational reference
- [Slash commands reference](../reference/slash-commands.md) — `/auth`, `/chat`, `/providers`
