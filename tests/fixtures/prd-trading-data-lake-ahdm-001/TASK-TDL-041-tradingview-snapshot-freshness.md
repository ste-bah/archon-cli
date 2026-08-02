# TASK-TDL-041 — TradingView Snapshot + Freshness

```yaml
task_id: TASK-TDL-041
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: TradingView Snapshot + Freshness
workstream: W2 Providers + Coverage
complexity: medium
status: pending
depends_on: ['TASK-TDL-040']
blocks: ['TASK-TDL-080']
source_sections: ['7', '8.4', '25.1']
required_env_keys: []
required_tools: [tv_health_check, chart_get_state, quote_get]
deliverable_contracts:
  - kind: provider_snapshot
    artifact_path: .archon/trading-lab/data/snapshots/<provider>/<symbol>.json
```

## Purpose

Persist current TradingView snapshots with provider timestamp metadata and the PRD staleness contract. Split out of TASK-TDL-040, which bundled snapshot capture with historical ingest, the CLI surface and TUI routing — five separable jobs whose combined remediation could not complete within one task's attempt budget.

## Scope

### In

- TradingView MCP status/state checks before snapshot capture.
- Snapshot storage under `.archon/trading-lab/data/snapshots/<provider>/<symbol>.json` with provider timestamp and captured-at metadata.
- PRD 5-minute stale classification applied consistently to captured snapshots.
- `data snapshot --provider tradingview` provider path.

### Out

- Historical OHLCV ingest, paging and dataset registry entries (TASK-TDL-040 owns those).
- Clap argument definitions and TUI alias routing (TASK-TDL-042 owns those).
- Synthesizing snapshot values when MCP returns nothing — fail closed with captured evidence.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_store.rs`, **`src/command/trading_data_provider/tradingview.rs`** (the TradingView MCP implementation — REQUIRED write scope), `src/command/trading_data.rs`, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- REAL LIVE MCP CAPTURE REQUIRED: the delivered snapshot MUST originate from an actual live invocation of the declared MCP tools this run, evidenced by captured provider timestamp metadata. A mocked or fixture snapshot does NOT satisfy the deliverable.
- Snapshot artifacts include provider timestamp and captured-at metadata.
- Snapshots are classified stale after 5 minutes per the PRD contract.
- Snapshot capture failure is reported honestly with the captured provider error; no placeholder snapshot is written.
- If the MCP/session is unreachable, BLOCK HONESTLY with captured failure evidence rather than writing a synthesized snapshot.

## Focused Tests

- Snapshot artifact persistence test.
- Snapshot stale-after-5-min test.
- Snapshot capture failure returns actionable unavailable evidence.

## Adversarial Review Notes

- Verify the snapshot timestamp is provider-sourced, not locally generated at write time.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `needs_review` (semantic)

> TASK-TDL-041 is not accepted. Live TradingView MCP tools were reachable and a BATS:AAPL snapshot artifact with provider timestamp exists, but the implementation/test evidence does not prove the delivered CLI path created a live MCP-derived snapshot this run, focused tests use fixtures, and repository changes include out-of-scope TradingView OHLCV/provider work beyond snapshot freshness.

**Unremediated findings against this task (7, 5 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `F1` · high · adversarial review**

- *claim:* Delivered snapshot MUST originate from actual live invocation of declared MCP tools this run.
- *evidence:* snapshot.rs live path runs local Node CLI commands status/state/quote; verification separately ran MCP tools, then ran CLI. Artifact cannot by itself prove MCP-tool provenance.
- *sources:* src/command/trading_data/snapshot.rs:66-73; verification-wave-verify-task-tdl-041-3-32-0.json
- *status:* risk

**F2 · `F2` · medium · adversarial review**

- *claim:* Focused tests verify persistence, freshness, and fail-closed behavior for the deliverable.
- *evidence:* All focused snapshot tests set ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE, so they verify fixture parsing and not live provider behavior.
- *sources:* src/command/trading_data_tests.rs:254-354
- *status:* falsifies_test_strength

**F3 · `F3` · medium · adversarial review**

- *claim:* TASK-TDL-041 required TradingView MCP implementation write scope was satisfied.
- *evidence:* Task file names src/command/trading_data_provider/tradingview.rs as REQUIRED write scope; implement and remediate manifests show identical pre/post hash for that file.
- *sources:* TASK-TDL-041-tradingview-snapshot-freshness.md:40-43; implement/remediate patch manifests
- *status:* falsifies_scope_claim

**F4 · `F4` · medium · adversarial review**

- *claim:* Accepted remediation is fully verified.
- *evidence:* Remediation commands record cargo fmt --all -- --check failed with exit_code 1.
- *sources:* remediate-task-tdl-041-3-31-0.json:128-133
- *status:* quality_gate_failed

**F5 · `GAP-TDL041-REQUIRED-WRITE-SCOPE` · medium · coverage review**

- *source:* /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-041-tradingview-snapshot-freshness.md:40-43

**F6 · `F5` · low · adversarial review**

- *claim:* Snapshot includes provider timestamp and captured-at metadata and is stale after 5 minutes.
- *evidence:* Artifact has captured_at 1785289801 and provider_timestamp 1785245400, a delta far greater than 300 seconds, with freshness Stale/stale.
- *sources:* BATS_AAPL.json:2-12; BATS_AAPL.json:188-197
- *status:* supported

**F7 · `F6` · low · adversarial review**

- *claim:* TradingView failure path does not write placeholder snapshot on capture failure.
- *evidence:* tradingview_snapshot returns tradingview_unavailable on payload error; tradingview_unavailable renders unavailable JSON and does not call persist_snapshot.
- *sources:* src/command/trading_data/snapshot.rs:23-29; src/command/trading_data/snapshot.rs:156-169
- *status:* supported

**Cross-task findings naming this task (2).**

Authored by the review reduce stage, which sees all tasks at once. No single task owns these;
they are context. Address the part that lies inside your declared scope and say plainly what
belongs elsewhere — do not attempt the whole pattern from here.

**CROSS-TASK — F1 · `REDUCE-XTASK-004` · medium · adversarial review**

- *summary:* Multiple task verifications overstate test coverage by testing weaker paths than the acceptance criteria require.
- *evidence:* TDL020-ADV-002: slash routing parser test covers list/export only, not required validate or validate-ohlcv.; TDL020-ADV-004: invalid duplicate/OHLC/volume focused tests inspect validation_report directly rather than end-to-end CLI validation of persisted invalid JSONL.; TDL041 F2: focused snapshot tests use ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE and do not verify live provider behavior.; F-TDL110-004: focused test suite validates non-AHDM fallback instead of rejecting it.; Later Pine F5: validation script checks marker strings and JSON key presence only, not semantic risk-analysis correctness.
- *impact:* Tests can pass while business-critical routing, live data provenance, AHDM semantics, or review quality are broken.
- *status:* open
- *recommended action:* Add negative and end-to-end tests that fail when the asserted acceptance semantics are weakened.

**CROSS-TASK — F2 · `REDUCE-XTASK-006` · medium · adversarial review**

- *summary:* Provider availability evidence is inconsistent across capability, snapshot, and TradingView/Pine tasks.
- *evidence:* F-TDL030-001: the same polygon capability record reports credential_state=missing and provider_env_proof credential_state=present.; F-TDL030-002: stooq record has credential_state="" while provider_env_proof.credential_state=present.; TDL041 F1: delivered TradingView snapshot artifact cannot by itself prove declared MCP-tool provenance because live path runs local Node CLI commands while verification separately ran MCP tools.; Later Pine F1: several declared TradingView/Pine tools failed due Pine Editor unavailable even though tool exercise and acceptance were claimed.
- *impact:* Operators cannot reliably distinguish missing credentials, unavailable tools, local CLI fallback, fixture evidence, and real provider success.
- *status:* open
- *recommended action:* Normalize provider state labels and record per-artifact provenance binding each output to the exact provider/tool invocation that produced it.

<!-- PRIOR-RUN-FINDINGS:END -->

## Required Task Checklist

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
