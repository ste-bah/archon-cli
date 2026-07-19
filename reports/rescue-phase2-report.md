# Workflow Runtime Rescue — Phase 2 Report: Un-JS the PRD Scaffold

Branch: `rescue/workflow-runtime-v3`
Result: the decomposed-PRD lifecycle executes natively in Rust. QuickJS
interprets only LLM-authored scripts (`RunTemplate`). The string-surgery
lifecycle splice functions are deleted.

## What changed

- `WorkflowV2ScriptRunner::run_decomposed_lifecycle` (new): a Rust driver that
  issues the same host calls, in the same order, with the same ids, options,
  and verbatim prompts as the scaffold JS — through the same
  `WorkflowScriptHost::execute` entry point the QuickJS bridge used. Result
  reuse, `dynamic_wave_source_metadata`, input hashing, run-control polling,
  events/TUI, call records, and terminal semantics are bit-for-bit the same
  code paths (keep-list preserved).
- Modules: `workflow_live_v2_lifecycle{,_waves,_impl,_verify,_review}.rs`
  (driver + wave loop + implementation/remediation/ownership + verification/
  triage + review/final gates), `workflow_live_v2_lifecycle_prompts.rs`
  (every scaffold `task:` string, verbatim), and
  `workflow_live_generated_lifecycle_{support,outcomes,remediation}.rs`
  (faithful ports of the JS helpers, delegating to the pre-existing Rust
  contract twin in `workflow_live_generated_contract*.rs`, which the contract
  test suites already pin).
- Routing: `execute_generated_v2_run` sends runs with a task universe to the
  Rust lifecycle; saved templates still go through QuickJS.
- **Deleted:** `apply_verification_remediation_lifecycle` and
  `apply_ownership_expansion_lifecycle` (the find-marker/splice-constant
  string surgery, `workflow_live_generated_scaffold_{verification,ownership}.rs`).
  Their JS payloads are baked textually into `body_a.js`; the generated
  scaffold is byte-identical (hash-stable; all semantics marker tests pass
  unchanged).

## Gate results

- Decomposed-PRD integration tests: all pass against the Rust implementation.
  288 workflow-scoped bin tests green; the only red are
  `canary_wf_afae6bee_regression` (intentionally red until Phase 4) and the
  pre-existing `generated_v2_read_only_verification_branch_stays_read_only`.
- The canary reproduces its pre-port execution trace call-for-call (same call
  ids, statuses, branch findings, and `blocked-verification-failed-1`
  terminal state) — the strongest available behavioral-equivalence evidence.
- FileSizeGuard: green.

## Approved deviation (maintainer decision 2026-07-07: Option B)

The handover's "net LOC strongly negative" gate for Phase 2 is deferred: the
scaffold JS (body_a/body_b/noop/remediation + the 669-line JS contract twin)
is now generation-and-record only — never executed for decomposed runs — but
still serves as the recorded run artifact and hash identity. Deleting it
requires swapping the recorded artifact for a Rust-rendered plan descriptor
(approval-surface and bundle-format change), which the maintainer approved
folding into Phase 5, where the semantics substring validators die in the
same stroke. Phase 2 LOC: roughly +2.5k Rust / −0.4k (splices + trims); the
offsetting ~3.4k deletion (scaffold JS + semantics validators + their tests)
moves to Phase 5.

## Follow-ups carried forward

- Phase 5: recorded-artifact descriptor swap + scaffold JS + JS contract twin
  + semantics substring validators deletion.
- The current-thread-runtime-in-spawn_blocking threading model is untouched
  per the handover's landmine list.
