# TASK-TDL-050 — OpenBB/Polygon Native Ingest

```yaml
task_id: TASK-TDL-050
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: OpenBB/Polygon Native Ingest
workstream: W2 Providers + Coverage
complexity: large
status: blocked
depends_on: ['TASK-TDL-030']
blocks: ['TASK-TDL-080']
source_sections: ['7', '8.4', '25.2']
implements: [REQ-DL-034]
required_env_keys: [OPENBB_API_URL, POLYGON_API_KEY]
required_tools: []
deliverable_contracts:
  - kind: native_dataset_manifest
    artifact_path: .archon/trading-lab/data/datasets/<dataset-id>/<version>/manifest.json
  - kind: trading_data_registry_entry
    artifact_path: .archon/trading-lab/data/registry.json
```

## Purpose

Implement OpenBB/Polygon native OHLCV ingestion where credentials and exact intervals support it.

## Scope

### In

- Credential presence detection before full fetch.
- Exact native interval fetch.
- Raw request/response metadata with secrets redacted.
- Complete dataset artifact contract from TASK-TDL-010, including `manifest.json`, `ohlcv.jsonl`, `validation.json`, `raw/request.json`, `raw/headers.redacted.json`, `raw/response.<json|csv|zip|txt>`, and `raw/provider-notes.md`, or explicit fail-closed residual gaps for unavailable artifacts.
- Adjustment/session metadata when available.
- CLI/TUI provider path for `data fetch-native --provider polygon` or `--provider openbb`, using exact native intervals only.

- Full PRD fetch command signature: `archon trading data fetch-native --provider <PROVIDER> --symbol <SYMBOL> --timeframe <TF> --start <RFC3339|YYYY-MM-DD> --end <RFC3339|YYYY-MM-DD> --dataset-id <ID> [--target <PROJECT>]`.
- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.

### Out

- Using OpenBB/yfinance data as Polygon data.
- Credential storage in artifacts.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, **the OpenBB/Polygon provider implementation modules `src/command/trading_data_provider_openbb.rs`, `..._openbb_request.rs`, `..._openbb_http.rs`, `..._openbb_parse.rs`, `..._openbb_metadata.rs` (REQUIRED write scope for this task)**, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.
- KNOWN DEFECTS TO FIX IN THIS TASK: (a) the success path sets `expected_bars` from the OBSERVED bar count, which is circular and can never detect a short fetch — derive expected bars from the requested span + timeframe and compare honestly; (b) non-yfinance providers are marked `production_eligible` without proving requested-span coverage; (c) `src/cli_args/tests.rs` has `fetch-native` parse tests for yfinance and stooq only — add the missing polygon/openbb parse contract tests. A previous run diagnosed these correctly but could not act because the provider modules were outside the declared write scope — the scope above now includes them.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- REAL LIVE FETCH REQUIRED (production deliverable — not a fixture): the delivered native dataset MUST originate from an actual live provider fetch performed this run, evidenced by captured `raw/request.json`, `raw/response.<json|csv|zip|txt>`, and `raw/provider-notes.md` proving a real call returned real native candles. Fixture/mock tests may validate parse, normalization, and redaction logic but MUST NOT satisfy the dataset deliverable and MUST NOT back a `production_eligible=true`/Healthy registry entry.
- REAL VOLUME REQUIRED, NOT A PROBE: the delivered production dataset's bar count MUST be consistent with the requested `--start`/`--end` span (a multi-year daily request means hundreds+ bars, not a handful). A token/probe fetch proves connectivity ONLY — it is diagnostic evidence and MUST NOT be recorded `production_eligible=true`/Healthy or satisfy the dataset deliverable. State the requested span and returned bar count explicitly; a material shortfall is a residual gap and keeps the dataset non-production-eligible.
- No fixture, synthetic, or placeholder dataset may be recorded `production_eligible=true`/Healthy. Absent captured live-fetch evidence the registry entry MUST be `production_eligible=false`.
- If the provider is unreachable (missing/invalid credentials, blocked, rate-limited), BLOCK HONESTLY with the captured provider-failure evidence; do NOT mint a placeholder or fixture-derived dataset to pass acceptance. An empty/unavailable result is valid only with captured provider-failure evidence.
- PLAN ENTITLEMENT IS NOT UNAVAILABILITY — FETCH WHAT THE PLAN ALLOWS: a subscription/plan rejection (e.g. HTTP 403 `NOT_AUTHORIZED` with a "plan doesn't include this data timeframe" message) means the credential is VALID but the requested window exceeds the entitlement. Do NOT treat this as provider-unavailable and do NOT block outright. Narrow the request to the maximum entitled window (bisect or step the `--start` forward), ingest the real candles that ARE entitled, and record the entitlement boundary as an explicit residual gap naming the provider message and the earliest date actually served. Only a request that fails at EVERY window is a true unavailable.
- REQUIRE THE LOCAL PROVIDER SERVICE TO BE UP FIRST: when the provider is reached through a local service (e.g. `OPENBB_API_URL`), verify that endpoint responds before concluding anything about credentials. A connection failure to the local service is a SERVICE-DOWN residual gap (start it via `scripts/start-openbb-api.sh` with the environment loaded), never evidence that the provider or credential is unavailable.
- AC-DL-003 and AC-DL-007 for OpenBB/Polygon path.
- Missing credentials return exact unavailable reason.
- Adjusted/unadjusted policy is recorded.
- Provider identity is never mislabelled.
- Successful ingest writes a v2 registry entry pointing to metadata, validation report, manifest, normalized JSONL, and raw artifacts; interrupted ingest must not leave a healthy registry entry.
- Clap parsing and command dispatch exist for every owned CLI command, with matching TUI slash alias routing where required.
- CLI parse tests require all mandatory `fetch-native` flags: `--provider`, `--symbol`, `--timeframe`, `--start`, `--end`, and `--dataset-id`; `--target` remains optional.

## Focused Tests

- Missing credential capability test.
- Redaction test.
- Native interval response normalization test with fixture.
- CLI parse and TUI slash-alias routing tests for every owned command.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `needs_review` (semantic)

> Verification cannot accept TASK-TDL-050. Code and artifacts show substantial implementation evidence, but current source does not reproduce the inspected production Polygon artifact semantics: daily expected_bars is computed as calendar days while the live artifact uses trading-session count, and artifact metadata incorrectly records credential_required=false for a credentialed Polygon/OpenBB fetch.

**Findings:** none. The mandatory reviews inspect only ACCEPTED tasks, so a task that was
never accepted produced no findings. Absence of findings here is absence of review, not a
clean bill of health.

<!-- PRIOR-RUN-FINDINGS:END -->

## Required Task Checklist

- implements (normative requirement IDs)
- scope
- files expected to change
- files forbidden to change
- acceptance criteria
- focused tests
- line-count check
- complexity check where applicable
- adversarial review notes
- explicit residual gaps with fail-closed behavior

## Global Constraints

- Keep changed/new files under 500 lines where possible.
- No hardcoded secrets or provider credentials.
- No production candle resampling.
- No vague "later", "TBD", "probably", or "best effort" without a residual gap record.
