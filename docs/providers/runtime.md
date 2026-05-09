# Provider Runtime

Archon provider runtime state is persisted in the governed Cozo learning store
by default. Temporary caches may still use cache directories, but durable
provider profile, fallback, status, and rate-limit evidence belongs in Cozo.

## Auth Profiles

Import local credentials into the profile store:

```bash
archon providers profiles import
```

Inspect runtime selection and skip reasons:

```bash
archon providers profiles select anthropic --auth-kind oauth
archon providers profiles select openai-codex
```

The selector orders healthy profiles first, honors cooldown and disabled
markers, and reports explicit reasons such as `cooldown`, `disabled`, or
`auth-kind-mismatch`. Runtime request events include the selected profile id
when one is known.

## Status And Limits

```bash
archon providers status
archon providers limits --provider openai-codex
```

`providers status` enriches local provider status with the selected Cozo auth
profile and persists redacted snapshots. `providers limits` shows observed
rate/usage windows captured from real provider failures.

## Codex Strategy

Codex is configured under `[providers.openai-codex]`:

```toml
runtime = "direct" # direct | auto | app_server
direct_fallback = false
app_server_discovery_timeout_ms = 2500
```

`direct` preserves Archon's existing Codex backend path. `app_server` fails
visibly until a real adapter is implemented. `auto` may fall back from
app-server to direct only when `direct_fallback=true`; that fallback emits a
provider runtime event.

Anthropic Claude Code spoofing remains a protected compatibility contract and
is not controlled by Codex strategy settings.
