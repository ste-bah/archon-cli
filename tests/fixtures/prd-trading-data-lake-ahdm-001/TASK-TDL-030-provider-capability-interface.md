# TASK-TDL-030 — Provider Capability Interface

```yaml
task_id: TASK-TDL-030
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Provider Capability Interface
workstream: W2 Providers + Coverage
complexity: medium
status: blocked
depends_on: ['TASK-TDL-020']
blocks: ['TASK-TDL-040', 'TASK-TDL-050', 'TASK-TDL-060', 'TASK-TDL-070']
source_sections: ['8.4', '22', '25']
implements: [REQ-DL-030, REQ-DL-031, REQ-DL-032, REQ-DL-110, REQ-DL-111, REQ-DL-112, REQ-DL-113]
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: provider_capability_registry
    artifact_path: .archon/trading-lab/data/provider-capabilities.json
```

## Purpose

Define provider-neutral capability and native fetch contracts with exact unavailable reasons.

## Scope

### In

- `can_fetch_symbol_timeframe(provider, symbol, timeframe)`.
- `fetch_ohlcv_native(...)` trait/interface contract.
- `fetch_current_snapshot(...)` contract where supported.
- Capability result schema from PRD §22.
- CLI/TUI `data providers` and `data capability` commands with JSON output support where required.
- Generic PRD snapshot command shape: `archon trading data snapshot --provider <PROVIDER> --symbol <SYMBOL> [--target <PROJECT>]`; provider-specific tasks implement supported/unavailable behavior.
- Snapshot freshness contract: current snapshots must include provider/captured timestamp metadata; missing or older-than-5-min snapshots are stale and fail closed for any production/promotion decision.
- Persist provider capability results to `.archon/trading-lab/data/provider-capabilities.json` using the PRD §22 schema, with redacted-safe fields only, `checked_at`, unavailable reasons, and fail-closed `can_fetch=false` for blocks, credentials, unsupported intervals, or provider errors.

- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.

### Out

- Full provider implementations.
- Expensive full-download probes.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-DL-002 foundation exists.
- 401/403/404/missing credentials/missing native interval return `can_fetch=false`.
- Provider blocks are represented, not bypassed.
- Interface is provider-neutral.
- `.archon/trading-lab/data/provider-capabilities.json` is refreshed atomically and never treats missing/unknown capability fields as fetchable.
- Clap parsing and command dispatch exist for every owned CLI command, with matching TUI slash alias routing where required.
- `archon trading data snapshot --provider <PROVIDER> --symbol <SYMBOL> [--target <PROJECT>]` parses and dispatches generically before provider-specific support checks.
- Current snapshots are stale after 5 minutes; stale/missing snapshot state is reported explicitly and cannot satisfy production or promotion gates.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading provider_capability_parse` — capability result parsing.
- `cargo test -p archon-trading provider_unavailable_reason` — unavailable reason mapping.
- `cargo test -p archon-trading provider_no_expensive_download` — no expensive download on the capability path, against a mock.
- `cargo test -p archon-trading provider_capabilities_redaction` — provider capabilities persistence and secret redaction.
- `cargo test -p archon-trading snapshot_stale_after_five_minutes` — snapshot stale-after-5-min classification.
- `cargo test --bin archon trading_data_capabilities_parse` — CLI parse for every owned command.
- `cargo test --bin archon trading_data_snapshot_parse` — generic snapshot command parse/dispatch.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `accepted`

> TASK-TDL-030 is verified as already satisfied by existing code and artifacts. Provider capability interfaces, fail-closed capability states, atomic provider-capabilities persistence, CLI parse/dispatch, generic snapshot dispatch, stale snapshot classification, and TUI slash aliases are present and focused tests passed after correcting one timed-out broad test invocation.

**Unremediated findings against this task (6, 4 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `F-TDL030-001` · high · adversarial review**

- *claim falsified:* Declared provider capability registry artifact has honest, redacted-safe credential fields.
- *bounded evidence:* provider-capabilities.json:3-45 polygon:SPY:1D:missing_credentials has credential_state=missing, missing_credentials=true, unavailable_reason=missing provider credentials.; Same record provider_env_proof has credential_state=present and POLYGON_API_KEY/OPENBB_API_URL presence=present.; provider_methods.rs:249-356 enriches provider_env_proof from current environment, not persisted result state.
- *impact:* Operators may see simultaneous proof of credentials present and missing for the same probe, making unavailable reasons non-actionable.
- *status:* open

**F2 · `F-TDL030-002` · high · adversarial review**

- *claim falsified:* Provider capability records have valid explicit credential state.
- *bounded evidence:* provider-capabilities.json:327-377 stooq:SPY:1D:live_provider_blocked_tdl060_remediate_3_46 has credential_state="".; The same record has provider_env_proof.credential_state=present.
- *impact:* Empty credential_state appears outside the documented labels observed in code (present/missing/not_required), weakening schema consistency.
- *status:* open

**F3 · `F-TDL030-003` · medium · adversarial review**

- *claim falsified:* provider-capabilities.json is refreshed atomically as a coherent current artifact.
- *bounded evidence:* Top-level checked_at is 2026-07-29T03:26:51.611940+00:00.; Record checked_at values include 2026-07-18, 2026-07-20, 2026-07-24, and 2026-07-29.
- *impact:* A registry snapshot may be a historical accumulation rather than a freshly regenerated capability matrix; acceptance criteria line 63 says refreshed atomically.
- *status:* open

**F4 · `F-TDL030-004` · medium · adversarial review**

- *claim falsified:* TASK-TDL-030 is unambiguously accepted/completed in project evidence.
- *bounded evidence:* Task contract YAML line 10 says status: blocked.; Verification branch result status is accepted with residual_gaps=[].
- *impact:* Downstream workflow state and task source-of-truth disagree; dependent tasks may be blocked or proceed inconsistently.
- *status:* open

**F5 · `F-TDL030-005` · low · adversarial review**

- *claim falsified:* Implementation stage itself produced reproducible verification evidence.
- *bounded evidence:* Implementation manifest status is idempotent_noop with changed_files=[], created_files=[], deleted_files=[].; Implementation manifest verify_command is null.
- *impact:* Noop may be legitimate, but it means implementation-stage acceptance did not itself prove a task patch; later verification must carry the burden.
- *status:* open

**F6 · `F-TDL030-006` · low · adversarial review**

- *claim falsified:* All focused tests passed in the implementation stage.
- *bounded evidence:* Implementation-stage context reported cargo test -p archon-tui filter_trading_data_aliases timed out with exit code 124.; Verification-stage evidence later claims the same focused TUI test passed.
- *impact:* Original implementation evidence had a residual test gap; later verification appears to mitigate it if trusted.
- *status:* mitigated_by_later_verification

**Cross-task findings naming this task (4).**

Authored by the review reduce stage, which sees all tasks at once. No single task owns these;
they are context. Address the part that lies inside your declared scope and say plainly what
belongs elsewhere — do not attempt the whole pattern from here.

**CROSS-TASK — F1 · `REDUCE-XTASK-001` · high · adversarial review**

- *summary:* Multiple tasks are treated as accepted by workflow evidence while their task source metadata remains blocked.
- *evidence:* TDL020-ADV-003: TASK-TDL-020 task file lines 3-11 status: blocked while accepted verifier artifact status: accepted.; F-TDL030-004: Task contract YAML line 10 says status: blocked while verification branch result status is accepted with residual_gaps=[].; F-TDL110-003: TASK-TDL-110 remains status: blocked with depends_on TASK-TDL-100 while workflow acceptance treats it as accepted/noop.; TDL120 F2: Task spec YAML line 10 has status: blocked while TASK-TDL-120 accepted/completed by workflow.
- *impact:* Workflow consumers may derive conflicting execution order and readiness decisions depending on whether they trust task files or verifier artifacts.
- *status:* open
- *recommended action:* Define a single authoritative lifecycle source or update task metadata atomically as part of verified acceptance.

**CROSS-TASK — F2 · `REDUCE-XTASK-002` · high · adversarial review**

- *summary:* Several accepted/remediated results rely on no-op or empty patch provenance while claiming generated or updated deliverables.
- *evidence:* Initial audit F2: recorded generation command contains only a comment and patch file is zero-line/empty.; F-TDL030-005: implementation manifest is idempotent_noop with changed_files=[], created_files=[], deleted_files=[] and verify_command=null.; TDL120 F1: remediation evidence says compile-report.json was updated, but patch manifest has changed_files=[], created_files=[], deleted_files=[], status.idempotent_noop and patch file is zero lines.; Later Pine F2: fresh remediation accepted, but remediation patch manifest has changed_files=[], created_files=[], deleted_files=[] and status idempotent_noop.
- *impact:* Accepted artifacts may reflect pre-existing state, out-of-band writes, or unverifiable regeneration rather than the claimed in-branch implementation/remediation.
- *status:* open
- *recommended action:* Require each artifact-producing stage to record a reproducible command, content hash before/after, and non-empty artifact diff when claiming an update.

**CROSS-TASK — F3 · `REDUCE-XTASK-003` · high · adversarial review**

- *summary:* Accepted verification repeatedly reports residual_gaps=[] or satisfied acceptance while map findings document fail-closed residual gaps or unavailable required tooling.
- *evidence:* Initial F3: verification result has residual_gaps: [] while implementation result and artifact record four fail-closed residual gaps.; F-TDL030-004: verification branch accepted with residual_gaps=[] despite blocked task metadata and capability registry contradictions.; TDL120 F3: compile-report.json and accepted verification record Pine Editor MCP calls could not open, supporting exploratory-only status and blocking promotion.; Later Pine F1: required TradingView/Pine tools list has 7 entries, but four required tools failed due Pine Editor unavailable while acceptance criteria were claimed satisfied.
- *impact:* Downstream readiness decisions can overtrust accepted status even where evidence says the deliverable must remain exploratory, degraded, or blocked.
- *status:* open
- *recommended action:* Make fail-closed residual gaps first-class acceptance blockers or explicitly downgrade task status to needs_review/partial when required tools cannot run.

**CROSS-TASK — F4 · `REDUCE-XTASK-006` · medium · adversarial review**

- *summary:* Provider availability evidence is inconsistent across capability, snapshot, and TradingView/Pine tasks.
- *evidence:* F-TDL030-001: the same polygon capability record reports credential_state=missing and provider_env_proof credential_state=present.; F-TDL030-002: stooq record has credential_state="" while provider_env_proof.credential_state=present.; TDL041 F1: delivered TradingView snapshot artifact cannot by itself prove declared MCP-tool provenance because live path runs local Node CLI commands while verification separately ran MCP tools.; Later Pine F1: several declared TradingView/Pine tools failed due Pine Editor unavailable even though tool exercise and acceptance were claimed.
- *impact:* Operators cannot reliably distinguish missing credentials, unavailable tools, local CLI fallback, fixture evidence, and real provider success.
- *status:* open
- *recommended action:* Normalize provider state labels and record per-artifact provenance binding each output to the exact provider/tool invocation that produced it.

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
