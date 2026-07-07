# Workflow Runtime Rescue — Phase 0 Baseline

Branch: `rescue/workflow-runtime-v3` (from `review/workflow-runtime-current-state`, b62f1f8e)
Date: 2026-07-07
Reference: `HANDOVER-workflow-runtime-rescue.md`, PR #48 review.

## Baseline LOC (the demolition target)

| Surface | Lines |
|---|---|
| `crates/archon-workflow/src` | 34,336 |
| workflow files in `src/command` | 29,471 |
| workflow tests (`crates/archon-workflow/tests` + root) | 18,216 |
| **Runtime total (excl. tests)** | **63,807** |

Definition-of-done target: runtime total ≤ 10,000 (aim 5–8k).

## Pre-existing failures (NOT attributable to the rescue)

### `cargo check --workspace`
Clean (2m54s, `-j 4`).

### `cargo test -p archon-workflow --no-fail-fast`
12 failures across 6 test binaries, all in modules the rescue deletes:

| Test binary | Failures | Rescue phase that deletes the module |
|---|---|---|
| `generated_task_items` | 2 (`expected_target_files` SpecInvalid) | Phase 5 (`generated_*`) |
| `required_artifact_contract` | 1 (`generated_final_gate_records_candidate_artifacts_from_prd_layout`) | Phase 3 |
| `required_artifact_guidance` | 1 | Phase 3 |
| `required_artifact_repair_contract` | 4 | Phase 3 |
| `required_artifacts` | 1 (`blocked_artifact_repair_evidence_reaches_final_gate`) | Phase 3 |
| `runtime_continuation` | 3 (legacy remediation continuation) | Phase 4/5 |

The compensation layers the review flagged are already failing their own tests
on the review branch.

Additionally discovered during Phase 1 (the bin-crate suite was never fully run
at baseline because ci-gate fail-fasts at the file-size step):

- `command::workflow_live::workflow_live_runner_tests::generated_v2_read_only_verification_branch_stays_read_only`
  fails pre-rescue (verified against the untouched review branch): the
  text-sniffing `command_execution_stage` heuristic grants Bash to a read-only
  verification-inventory call. Same class as the `context_output` layer;
  Phase 5 scope.

### `scripts/ci-gate.sh`
Fails at the FileSizeGuard step (fail-fast; later steps did not run):

- `crates/archon-trading/src/data_store/data_store_tests.rs` — 955 lines (> 500, not allowlisted)
- `web/src/views/WorkflowPage.tsx` — 502 lines (> 500, not allowlisted)

Both pre-date the rescue and are outside the workflow runtime. Maintainer
decision needed: split, allowlist, or accept as a known-red gate during the
rescue (each phase will report `ci-gate --only` results for unaffected steps
plus the full run).

## Canary regression test

`wf-afae6bee` result-store artifacts are not on this machine (the run lived on
the maintainer's Mac under `/Volumes/Externalwork`). Per the handover fallback,
the fixture is reconstructed synthetically:

- Test: `canary_wf_afae6bee_regression` in
  `src/command/workflow_live_canary_tests.rs` (bin-crate test).
- Fixture: one-task decomposed PRD (`TASK-TDL-001`) whose task file declares
  artifact evidence at `.archon/artifacts/TASK-TDL-001/gap-audit.md`.
- Scripted agent client honesty rule: an implementation/remediation agent
  writes the artifact **iff its prompt names the artifact path**; the focused
  verification agent checks the filesystem; verification failures are not
  classified `actionable_implementation_failure` (mirroring the real run, where
  triage never routed to write remediation and verification-repair-plan 1-1 →
  1-3 looped to exhaustion — see `src/command/fixtures/wffe12_*.json`).
- Assertions (the Phase 4 gate): artifact exists (declared contract reached the
  agent), no `blocked-verification-failed` run-level latch, and a final report
  is produced either way.

### Empirical result on the current architecture (2026-07-07)

The canary reproduces the exact wf-afae6bee terminal state:
`implementation-wave-1` accepted → `verification-wave-1` rewritten to branch
findings → `verification-repair-plan-1-1/1-2/1-3` loop → `verification-wave-1-1/2/3`
fail identically → **`blocked-verification-failed-1`**. The failing assertion is
the right one: the artifact was never written because no implementing agent's
prompt named it.

Two additional mechanisms were confirmed while calibrating the scripted client:

1. **Prompt injection works only when items carry `artifact_requirements`.**
   When the inventory item declares the path, the host injects an absolute
   path mapping into the implementation prompt (`project_artifact_root`
   section) and the instructed agent run completes that step. The wf-afae6bee
   failure requires the requirement to exist only in the task pack (surfaced at
   verification), never on the implementation item — which is what the
   LLM-authored inventory in the real run produced.
2. **The `context_output` text-sniffing layer rewrites honest agent results.**
   An accepted result without non-empty `files_changed`-family AND
   `commands_run`-family AND completion-family fields is host-rewritten into a
   `needs_review` branch finding ("declares accepted status without required
   evidence fields", `context_output_parts/part_1.rs`). Verification branches
   that fail this heuristic loop through repair plans that can never fix them
   — a second, independent route to the same run-level block. This layer is on
   the Phase 5 delete list; Phase 3's declared contracts replace it.

## Architecture findings confirmed during baseline

- `apply_verification_remediation_lifecycle` (and the ownership variant) do
  literal string surgery: find start/end markers in generated JS, splice in a
  constant JS block (`workflow_live_generated_scaffold_verification.rs`).
- "Semantics validation" is substring matching against whitespace-compacted JS
  source, duplicated for both quote styles
  (`workflow_live_generated_semantics_verification.rs`).
- No test executed the decomposed-PRD scaffold end-to-end before the canary;
  all scaffold tests assert on source text, not behavior.
- Decomposed-PRD mode detection hardcodes the literal `"task-tdl"` (this
  specific PRD's prefix) in `requires_authoritative_task_universe`.
- `src/command/fixtures/` contains ~20 JSON snapshots of previously blocked
  runs, each memorializing a carve-out added after a failure — the
  compensation-layer accretion pattern the review describes.

## Environment note

WSL2 host: cargo defaults to `-j 32` on 11 GiB RAM and OOM-kills the VM.
All cargo invocations during the rescue use `-j 4` (or `CARGO_BUILD_JOBS=4`)
plus `nice -n 19`; `ci-gate.sh` already pins `--test-threads=2` for the same
reason (2026-04-11 incident note in the script header).
