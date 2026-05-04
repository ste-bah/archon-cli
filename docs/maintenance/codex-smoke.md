# Codex Daily Smoke Runbook

Archon runs a daily Codex smoke against a dedicated ChatGPT Plus test account.
The smoke exercises credential restore, OAuth refresh, `archon auth status`,
and `archon chat --provider openai-codex`.

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
model name observed in local Codex configuration.

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
When the smoke recovers, it comments and closes the issue.

## Local Dry Run

Run the wiremock-backed local CLI smoke:

```bash
CARGO_BUILD_JOBS=2 cargo test -p archon-cli-workspace -j1 --test cli_chat_codex_smoke_local -- --test-threads=1
```

This does not contact OpenAI. It verifies that the same `archon chat` command
shape used by CI reaches the Codex provider and parses a Responses SSE stream.
