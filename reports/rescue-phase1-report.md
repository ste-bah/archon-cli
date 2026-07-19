# Workflow Runtime Rescue — Phase 1 Report: Single Grammar

Branch: `rescue/workflow-runtime-v3`
Result: the hand-rolled character-walking harness parser/validator is deleted;
QuickJS (rquickjs) is the only interpreter of workflow script source text.

## LOC delta

`+443 / −4,160` (net **−3,717**) across 42 files.

Deleted outright:
- `crates/archon-workflow/src/v2/harness{,_lex,_collect,_parse_call,_safety,_options,_sources}.rs` (1,245 lines)
- Crate root `harness{,_host_calls,_stage,_parse_props,_parse_literals,_misc}.rs` (999 lines — the legacy `HarnessCompiler`; `start_with_harness` now bundles without source-scanning, its full removal is Phase 5 with the legacy executor)
- `tests/v2_harness_validation.rs` + `tests/v2_harness_validation_parts/`, `tests/harness_compile.rs`

## Replacements (the +443)

- `src/command/workflow_live_v2_script_dry_run.rs`: `dry_run_workflow_plan` —
  runs the script in a QuickJS realm against a recording host that returns
  stubbed accepted results. Syntax errors are validation errors with the engine
  diagnostic; duplicate call ids and `model`/`provider` payload keys are
  rejected in the recorder (host policy, not source scanning); 10s watchdog.
- `decomposed_prd_plan_calls()` in `workflow_live_generated_scaffold.rs`: the
  deterministic scaffold's approval plan is declared by the Rust generator that
  produces the scaffold — one entry per stage family with item kind, source,
  write mode, and `dynamic_id_prefix`. Pinned equal (by stage family) to the
  deleted parser's extraction before deletion, per the phase gate.
- Determinism prelude in `script_source()` (live + dry-run): `Math.random`,
  `Date.now()`, `Date()`, and argless `new Date()` throw with "pass timestamps
  via args". No substring blocklist — the bare QuickJS realm has no
  fetch/require/process/fs.

## The silent fallback is closed

`workflow_live.rs` RunTemplate: a saved workflow that fails the dry-run is now
a hard, reported error with the QuickJS diagnostic. The
`executor.start_with_harness` fallback arm (legacy static executor with
different semantics) is deleted.

## One source of truth changes

- Resume (`workflow_live_v2_run.rs`) and restart invalidation
  (`workflow_restart.rs`) now read the host-call manifest persisted with the
  run at approval time instead of re-parsing `workflow.js`. Runs without a
  manifest: resume re-plans via dry-run; restart falls back to per-call
  invalidation (the result store's input hashing re-executes stale dependents).
- Template save (`template.rs`) no longer scans source text (the sandbox is
  the engine); the dry-run at the run boundary is the validation.

## Test status

- Bin crate: 288 workflow-scoped tests pass. Remaining red:
  `canary_wf_afae6bee_regression` (intentionally red until Phase 4) and
  `generated_v2_read_only_verification_branch_stays_read_only` (pre-existing,
  verified against the untouched review branch; recorded in the Phase 0 report).
- `cargo test -p archon-workflow`: same 12 pre-existing failures as baseline,
  no new failures.
- FileSizeGuard: green (the fallback deletion brought `workflow_live.rs` back
  under 500).
- `cargo clippy` on touched crates: no errors (pre-existing warnings only).

## Behavior changes to be aware of

- Approval metadata for decomposed-PRD runs now lists ~62 stage families once
  each instead of ~70 hash-suffixed synthetic ids — write-capable stages and
  methods unchanged.
- The plan preview for LLM-authored scripts is a dry-run trace (the path the
  script takes against stubbed results), not a syntactic call inventory.
- `generated_semantics_rejects_non_isolated_generated_write_fanout` was
  replaced by `generated_scaffold_emits_only_worktree_isolated_write_fanout`:
  with no source parser, write isolation is asserted against the generator's
  output and the declared plan directly.
