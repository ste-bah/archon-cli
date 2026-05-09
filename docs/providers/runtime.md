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

OpenAI, Bedrock, Vertex, local, and OpenAI-compatible providers can be
constructed by chat, TUI, pipeline, and shared runtime-router surfaces without
first requiring Anthropic credentials. If their own construction fails, Archon
may still attempt the legacy Anthropic fallback where that compatibility path is
available, and the fallback is recorded as a provider runtime event.

## Status And Limits

```bash
archon providers status
archon providers report
archon providers limits --provider openai-codex
```

`providers status` enriches local provider status with the selected Cozo auth
profile and persists redacted snapshots. `providers limits` shows observed
rate/usage windows captured from real provider failures. `providers report`
combines current status, persisted runtime events, recent limits, and failure
counts into a redacted provider-health report; use `--json` for automation.

## Codex Strategy

Codex is configured under `[providers.openai-codex]`:

```toml
runtime = "direct" # direct | auto | app_server
direct_fallback = false
app_server_url = "http://127.0.0.1:11434/codex"
app_server_discovery_timeout_ms = 2500
app_server_model_catalog = ["gpt-5.5", "gpt-5.4"]
```

`direct` preserves Archon's existing Codex backend path. `app_server` fails
visibly until a real adapter is implemented. `auto` may fall back from
app-server to direct only when `direct_fallback=true`; that fallback emits a
provider runtime event. `ARCHON_CODEX_APP_SERVER_URL` overrides
`app_server_url` for local diagnostics.

`archon providers status openai-codex` reports whether an app-server endpoint is
configured, whether direct fallback is selected, and whether the adapter is
still pending. App-server metadata is redacted before it is persisted to the
Cozo learning store.

Anthropic Claude Code spoofing remains a protected compatibility contract and
is not controlled by Codex strategy settings.
