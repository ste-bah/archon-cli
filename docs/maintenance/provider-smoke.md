# Provider Smoke Runbook

Archon provider smoke tests are maintainer-triggered checks for auth, routing,
spoof identity, and provider capability drift. They must never run from a
scheduled GitHub Actions cron because live provider checks can consume paid
quota.

## Source Of Truth

- Credentials: `~/.archon/.credentials.json` and read-only Codex CLI fallback
  `~/.codex/auth.json`
- Provider capability matrix: `archon providers capabilities`
- Local/live diagnostics: `archon providers doctor` and `archon providers doctor --live`
- Auth status: `archon auth status`
- Live smoke transcript: `.archon/evidence/provider-smoke/<date>/`
- Agentic parity proof: focused cargo tests named in this runbook plus any
  maintainer-captured live transcripts

## Default Local Check

This is safe to run during normal development. It does not contact providers and
must not print tokens.

```bash
archon auth status
archon providers capabilities
archon providers doctor
```

Expected result:

- Anthropic OAuth/API-key/proxy state is summarized without secrets.
- Codex OAuth state and `ARCHON_CODEX_DISABLED` are summarized without secrets.
- Spoof identity status is explained for OAuth-backed surfaces.
- Unsupported surfaces are visible in the capability matrix. As of v0.1.45,
  `openai-codex` should show support for chat, TUI, streaming, tools,
  subagents, `/btw`, coding/research pipelines, and gametheory.

## Opt-In Live Check

Run this only when a maintainer explicitly wants endpoint reachability proof.
The command performs provider endpoint reachability checks after local credential
validation; it must not send, print, or log token values.

```bash
archon providers doctor --live
```

Expected result:

- Present, non-expired provider credentials trigger an endpoint reachability row.
- Missing, disabled, or expired providers are skipped with an actionable reason.
- Remediation text tells the operator which login/env/config step to take next.

## Required Scenarios

Record each scenario under `.archon/evidence/provider-smoke/<date>/` with the
command output and a note that no full token value appears in the transcript.

| Scenario | Setup | Expected result |
|---|---|---|
| Anthropic OAuth spoof | Login with `archon auth login --provider anthropic` or provide `sk-ant-oat-...` via `ANTHROPIC_API_KEY` | Doctor reports OAuth/spoof identity active; Anthropic-capable pipelines remain enabled. |
| Anthropic API key | Set a raw `sk-ant-api...` key in `ANTHROPIC_API_KEY` | Doctor reports API-key-shaped env credential; spoof identity is not required. |
| Anthropic-compatible proxy | Set `ANTHROPIC_BASE_URL` plus a compatible API key | Doctor reports custom Anthropic base URL; capability matrix still identifies proxy limitations. |
| Codex OAuth | Login with `archon auth login --provider openai-codex --accept-tos` or the official Codex CLI; set `[llm].provider = "openai-codex"` for agentic surfaces | Doctor reports Codex OAuth present; `archon chat --provider openai-codex "Say ARCHON_SMOKE_OK_42"` works; capability matrix marks tools/subagents/pipelines as supported. |
| Codex kill switch | Set `ARCHON_CODEX_DISABLED=1` | Doctor reports Codex disabled and Codex provider use fails before execution. |
| Expired token | Use a fixture credential file with an expired `expiresAt` | Doctor reports expired and skips live ping with a login remediation. |
| Unknown credential format | Set an invalid `ANTHROPIC_API_KEY` shape in a temporary shell | Doctor reports unknown format and does not echo the value. |

## Safe Local Test Commands

These tests mock token shapes and do not call live providers.

```bash
CARGO_BUILD_JOBS=2 cargo test -p archon-cli-workspace -j1 command::providers -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-cli-workspace -j1 --test finalisation_provider_capabilities -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-cli-workspace -j1 --test cli_chat_codex_smoke_local -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-core -j1 --test codex_subagent_provider_parity -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-llm -j1 --test finalisation_codex_agentic_tools -- --test-threads=1
```

## Agentic Parity Spot Check

Use this when validating a release candidate. These commands may spend live
provider quota, so run them manually and redact transcripts before sharing:

```bash
archon providers capabilities
archon auth status
archon chat --provider openai-codex "Say ARCHON_CODEX_PARITY_OK"

# With `[llm].provider = "openai-codex"`:
archon gametheory run "Assess a small marketplace incentive design" --classify-only
archon pipeline code "Add a tiny tested helper" --dry-run
archon pipeline research "Summarize the tradeoffs in this fixture" --dry-run
```

Source of truth is not the final paragraph. Inspect persisted state after any
non-dry-run command with the matching `status`, `inspect`, `completion trust`,
or provenance command.

## Failure Triage

1. Run `archon providers doctor` first to separate local credential/config
   failures from endpoint drift.
2. Run `archon providers doctor --live` only when local state is sane.
3. If Codex spoof posture drifted, compare the current request headers and update
   `crates/archon-llm/resources/codex-compat.json`.
4. If Anthropic OAuth spoofing fails with authentication or identity rejection,
   verify the Claude Code identity headers and OAuth beta state before changing
   pipeline code.
5. If a provider is unsupported for a surface, do not bypass the capability
   router; update the capability matrix only after an implementation and test
   prove support.
