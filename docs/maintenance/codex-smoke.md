# Codex Smoke Runbook

Archon keeps a manual Codex smoke workflow for maintainers with a dedicated
ChatGPT Plus test account. The smoke exercises credential restore, OAuth
refresh, `archon auth status`, `archon chat --provider openai-codex`, and the
release-candidate parity checks that prove Codex remains wired for agentic
surfaces.

This workflow is intentionally **not scheduled**. Do not add a GitHub Actions
`schedule`/`cron` trigger: live provider checks consume paid quota and must only
run when a maintainer explicitly starts them.

## Required GitHub Secrets

- `CODEX_TEST_REFRESH_TOKEN`: refresh token from `archon auth login --provider openai-codex --accept-tos`.
- `CODEX_TEST_ACCOUNT_ID`: the ChatGPT account id associated with the test account.

Use a dedicated project-owned account. Do not use an engineer's personal
account. Rotate the refresh token monthly.

## Manual Workflow Run

```bash
gh workflow run codex-smoke.yml
```

The expected model output is the literal marker `ARCHON_SMOKE_OK_42`. The
default smoke model is `gpt-5.4`, matching the current Codex CLI ChatGPT-account
model name observed in local Codex configuration. Keep the workflow
`workflow_dispatch` only; scheduled live runs are not allowed.

## Triage

If the workflow opens or updates a `codex-smoke-broken` issue:

1. Check whether `archon auth status` reports a restored Codex account.
2. Check whether the failure is auth, quota, endpoint drift, or response shape.
3. Compare openclaw for changes to `originator`, `User-Agent`, `OpenAI-Beta`,
   or the Responses request body.
4. Update `crates/archon-llm/resources/codex-compat.json` if only the spoof
   posture drifted.
5. Re-capture openclaw fixtures with `codex_capture_sanitize` if request or SSE
   structure changed.

The workflow comments on an existing open issue instead of creating duplicates.
When the smoke recovers, it comments and closes the issue. Because the workflow
is manual-only, a maintainer should close stale smoke issues only after a fresh
manual run proves recovery.

## Local Dry Run

Run the wiremock-backed and fake-provider local checks. These do not contact
OpenAI and are the right default for PR validation:

```bash
CARGO_BUILD_JOBS=2 cargo test -p archon-cli-workspace -j1 --test cli_chat_codex_smoke_local -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-cli-workspace -j1 --test finalisation_provider_capabilities -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-llm -j1 --test finalisation_codex_agentic_tools -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -p archon-core -j1 --test codex_subagent_provider_parity -- --test-threads=1
```

Together these verify that the `archon chat` command shape reaches the Codex
provider, the Responses-style tool loop parses and continues, and two
Codex-named subagents can run concurrently through the provider-neutral runner.

## Optional Live Agentic Smoke

Only run this by hand with a maintained test account:

```bash
archon auth status
archon providers doctor --live
archon providers capabilities
archon chat --provider openai-codex "Say ARCHON_CODEX_PARITY_OK"
```

For a full release transcript, set `[llm].provider = "openai-codex"` and run a
small fixture through `/btw`, `/run-agent`, `/archon-code`, `/archon-research`,
and `/gametheory`. Inspect source-of-truth rows after each workflow. Do not
commit transcripts until token-shaped strings have been redacted.
