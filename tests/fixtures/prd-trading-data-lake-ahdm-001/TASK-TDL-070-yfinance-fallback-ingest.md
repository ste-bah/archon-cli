# TASK-TDL-070 — yfinance Fallback Ingest

```yaml
task_id: TASK-TDL-070
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: yfinance Fallback Ingest
workstream: W2 Providers + Coverage
complexity: medium
status: blocked
depends_on: ['TASK-TDL-030']
blocks: ['TASK-TDL-080']
source_sections: ['7', '8.4', '25.4']
implements: [REQ-DL-036]
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: diagnostic_dataset_manifest
    artifact_path: .archon/trading-lab/data/datasets/<dataset-id>/<version>/manifest.json
  - kind: trading_data_registry_entry
    artifact_path: .archon/trading-lab/data/registry.json
```

## Purpose

Implement yfinance fallback ingestion with degraded metadata and promotion restrictions.

## Scope

### In

- yfinance provider identity.
- Yahoo symbol/interval limitation metadata.
- Complete dataset artifact contract from TASK-TDL-010 for fallback ingest, including `manifest.json`, `ohlcv.jsonl`, `validation.json`, `raw/request.json`, `raw/headers.redacted.json`, `raw/response.<json|csv|zip|txt>`, and `raw/provider-notes.md`, or explicit fail-closed residual gaps for unavailable artifacts.
- Degraded quality status by default.
- yfinance datasets are always degraded/fallback and never satisfy promotion gates. A human-approved diagnostic override may allow exploratory reports only, marked `diagnostic=true`, and must list every overridden issue.
- CLI/TUI provider path for `data fetch-native --provider yfinance` with degraded/fallback metadata.

- Full PRD fetch command signature: `archon trading data fetch-native --provider <PROVIDER> --symbol <SYMBOL> --timeframe <TF> --start <RFC3339|YYYY-MM-DD> --end <RFC3339|YYYY-MM-DD> --dataset-id <ID> [--target <PROJECT>]`.
- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.

### Out

- Presenting yfinance as Polygon/OpenBB data.
- Automatic promotion eligibility for fallback data.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- REAL LIVE FALLBACK FETCH REQUIRED (not a fixture): the delivered yfinance dataset MUST originate from an actual live yfinance fetch this run, evidenced by captured `raw/request.json`, `raw/response.<json|csv|zip|txt>`, and `raw/provider-notes.md`. Fixture/mock tests may validate fallback metadata and interval mapping only; they MUST NOT satisfy the dataset deliverable. The dataset remains degraded / diagnostic-ineligible-for-promotion by design — but it must be a REAL fetch, never synthetic or placeholder. If yfinance is unreachable, block honestly with the captured failure rather than minting a fixture dataset.
- REAL VOLUME REQUIRED, NOT A PROBE: the delivered fallback dataset's bar count MUST be consistent with the requested `--start`/`--end` span. A token/probe fetch proves connectivity ONLY and does not satisfy the dataset deliverable. State the requested span and returned bar count explicitly; a material shortfall is a residual gap. (The dataset remains degraded / ineligible for promotion by design.)
- AC-DL-007 for yfinance fallback path.
- yfinance datasets are degraded by default.
- yfinance datasets never satisfy promotion gates, even when `--diagnostic-allow-degraded-data` is used for exploratory reports.
- Successful fallback ingest writes a v2 registry entry marked degraded/diagnostic-ineligible-for-promotion and pointing to metadata, validation report, manifest, normalized JSONL, and raw artifacts; interrupted ingest must not leave a healthy registry entry.
- Clap parsing and command dispatch exist for every owned CLI command, with matching TUI slash alias routing where required.
- CLI parse tests require all mandatory `fetch-native` flags: `--provider`, `--symbol`, `--timeframe`, `--start`, `--end`, and `--dataset-id`; `--target` remains optional.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading fallback_metadata` — fallback metadata.
- `cargo test -p archon-trading fallback_promotion_refusal` — promotion refusal for a degraded fallback dataset.
- `cargo test -p archon-trading fallback_interval_limitation` — interval limitation mapping.
- `cargo test --bin archon fetch_native_yfinance_parse` — CLI parse and TUI slash-alias routing for every owned command.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `accepted`

> Verified TASK-TDL-070 yfinance fallback ingest implementation, live dataset artifact contract, degraded/non-promotion behavior, CLI parsing, and TUI slash alias routing with concrete source, artifact, and focused test evidence. No files were changed by this verification run.

**Unremediated findings against this task (5, 1 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `finding-tdl070-001` · high · adversarial review**

- *claim falsified:* TASK-TDL-070 focused tests/adversarial notes require interval limitation mapping and no weakening of native-candle enforcement.
- *title:* 4H/240 yfinance fetch can be mislabeled as native interval
- *evidence:* {'lines': '135-146', 'path': '/Volumes/Externalwork/archon-cli/archon-cli/src/command/trading_data/yfinance.rs', 'summary': 'yahoo_interval maps 4H, 4h, and 240 to Yahoo interval 1h.'}; {'lines': '211-260', 'path': '/Volumes/Externalwork/archon-cli/archon-cli/src/command/trading_data/yfinance.rs', 'summary': 'metadata records the requested timeframe and unconditionally sets native_interval=true.'}; {'lines': '74-79', 'path': '/Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-070-yfinance-fallback-ingest.md', 'summary': 'Adversarial review notes explicitly require verifying the task does not weaken native-candle enforcement.'}
- *impact:* A user requesting --timeframe 4H or 240 could receive 1h Yahoo bars stored as a native 4H/240 dataset. That is a native-candle contract contradiction even if the dataset remains degraded and non-promotable.
- *status:* needs_review
- *recommended remediation:* Fail closed for non-native yfinance intervals such as 4H/240, or explicitly aggregate/label as non-native with native_interval=false and validation failure; add a focused test that rejects or flags 4H/240.

**F2 · `finding-tdl070-002` · info · adversarial review**

- *title:* Accepted SPY 1D artifact is degraded and non-production in registry
- *evidence:* {'lines': '2343-2371', 'path': '/Volumes/Externalwork/archon-cli/project-1/.archon/trading-lab/data/registry.json', 'summary': 'Registry entry has provider=yfinance, bars=4, status=Degraded, production_eligible=false, and artifact paths.'}
- *impact:* This supports, rather than falsifies, the accepted claim for the specific delivered SPY 1D dataset.
- *status:* not_falsified

**F3 · `TDL070-UNCOVERED-UNREACHABLE-YFINANCE-BLOCK` · ? · coverage review**

- *summary:* Accepted evidence covers only the successful live fetch path; no failure-path proof for unreachable yfinance is shown.
- *source:* /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-070-yfinance-fallback-ingest.md:58

**F4 · `TDL070-UNCOVERED-INTERVAL-LIMITATION-METADATA` · ? · coverage review**

- *summary:* Source maps 4H/240 to Yahoo 1h while metadata still sets native_interval=true, contradicting honest interval limitation/native interval semantics.
- *source:* /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-070-yfinance-fallback-ingest.md:30,71,76

**F5 · `TDL070-UNCOVERED-CHECKLIST-LINE-COMPLEXITY` · ? · coverage review**

- *summary:* Accepted evidence does not report line-count or complexity checks.
- *source:* /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-070-yfinance-fallback-ingest.md:81-90

**Cross-task findings naming this task (1).**

Authored by the review reduce stage, which sees all tasks at once. No single task owns these;
they are context. Address the part that lies inside your declared scope and say plainly what
belongs elsewhere — do not attempt the whole pattern from here.

**CROSS-TASK — F1 · `REDUCE-XTASK-005` · high · adversarial review**

- *summary:* Several accepted data-lake artifacts weaken source-of-truth or contract semantics while still being presented as compliant evidence.
- *evidence:* Initial F4: wildcard/template storage paths are marked present as 'Observed or contract-required' rather than direct physical evidence.; finding-tdl070-001: 4H/240 yfinance requests can receive 1h Yahoo bars stored as native 4H/240 datasets.; F-TDL110-001: StrategySpec claims loose_file_references_allowed=false while dataset_refs embed manifest_path, normalized_path, raw_path, and validation_path.; F-TDL110-002: AHDM dataset selection falls back to unrelated native production-eligible records when AHDM coverage is missing.
- *impact:* The implementation can appear to satisfy registry/storage/strategy contracts while substituting templates, non-native intervals, loose files, or unrelated datasets.
- *status:* open
- *recommended action:* Fail closed when physical dataset evidence or exact semantic coverage is absent; prohibit contract-required placeholders from counting as observed evidence.

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
