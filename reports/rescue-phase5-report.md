# Workflow Runtime Rescue — Phase 5 Report: Legacy Demolition

Branch: `rescue/workflow-runtime-v3`
Result: the legacy static executor, its control plane, the scaffold JS record,
and the semantics substring validators are gone. **Every workflow test in the
repository is green — zero known-red tests remain**, including the two
pre-existing failures that survived Phases 0–4.

## Deleted

**Execution entry points (src/command):** `RunSpec`, harness-less template
runs, and non-V2 resume/continue return clear errors instead of routing to the
legacy static executor; the deterministic CLI smoke mode is gone.

**The legacy executor family (crate):** `executor.rs` (`WorkflowExecutor` /
`ExecutionReport`), `executor_stage`, `executor_live*`, `executor_fanout*` (+
parts), `exec_state`, the `generated_*` spec patchers (completion, quality,
remediation, remediation_contract, sanitize, task_items, parts),
`remediation_inventory`, `remediation_noop`, `spec_write_coordination`,
`cargo`-unused `context.rs` stage builder, the legacy world-model report hook,
and 38 legacy test binaries — including the five pre-existing failures
(`generated_task_items` ×2, `runtime_continuation` ×3).

**The scaffold JS record (Phase 2's Option B deferral):** the four scaffold
`.js` bodies, the 669-line JS contract twin
(`workflow_live_generated_contract{.js,_retry.js,_preflight.js}`), and the
semantics substring validators
(`workflow_live_generated_semantics{,_support,_verification}.rs` + four test
files). The recorded run document is now a compact deterministic descriptor
(plan header + stage families + task universe + learning context) rendered by
`decomposed_prd_scaffold`; hash identity and reuse semantics preserved for old
runs (their recorded files keep their original hashes).

**The last text-sniffing tool-assignment bug:** typed V2 calls now decide
shell access from declared fields (focused-verification waves run commands;
all other read-only calls get no shell), fixing the pre-existing
`generated_v2_read_only_verification_branch_stays_read_only` failure.

## Kept despite stale "legacy-only" labels (live V2 dependencies, verified)

`context`/`context_output*` (write-branch output validation), `request`,
`completion_proof`, `remediation_items`, `executor_output`,
`spec_deser`/`spec_inference`/`spec_policy`/`spec_work_units` (spec-model
structure/validation), `work_unit_coverage`/`work_unit_gate` (write
coordinator), `cargo_target_env` (command execution). The write coordinator
runs agents through this chain today; the lib.rs comments claiming these were
legacy-only were wrong. Excising the `context_output` heuristics from the V2
write path is real behavior work, flagged as a follow-up with the canary as
the guard.

## Final LOC

| Surface | Baseline (Phase 0) | Now |
|---|---|---|
| `crates/archon-workflow/src` (non-test) | ~31k | 21,616 |
| workflow files in `src/command` (non-test) | ~26k | 20,430 |
| workflow tests (all) | ~21k | 17,010 |
| scaffold JS | ~2.3k lines | 0 |

Total (with tests): 63,807 → ~54,700; non-test runtime ≈ 42k. The handover's
5–10k target is **not reachable without deleting keep-list machinery**: the
result store, scheduler, write coordinator, agent adapter, host, contract
normalization, and native lifecycle are the remaining mass, and each is on the
keep list or is the Phase 2 replacement. The deletable legacy is gone; what
remains is the working runtime and its tests.

## Gates

- Full workflow test suite: **green with zero exceptions** (257 bin workflow
  tests incl. the canary; full `archon-workflow` crate suite).
- `cargo clippy -p archon-workflow -p archon-cli-workspace`: clean.
- FileSizeGuard: green.
- Definition-of-done greps: `harness_lex|harness_parse_call|harness_collect|
  start_with_harness|terminal_latched` → the only match is a guard test
  asserting the absence of `start_with_harness`.
- Full-workspace `ci-gate.sh` remains blocked by pre-existing clippy debt in
  non-rescue crates (archon-core 29 errors, archon-mcp 2 — recorded in the
  Phase 0 baseline report since the rescue's start).

## Follow-ups (out of rescue scope)

- `context_output` prose heuristics inside the V2 write-branch validation.
- Remaining mechanically-split `_tests_{a..f}`/`_parts` files: within the
  file-size gate; merging is cosmetic polish.
- The current-thread-runtime-in-spawn_blocking threading model (handover
  landmine list; untouched by design).
- World-model ingest for V2 runs (the legacy hook never fired for V2; deleted
  with the executor — re-add against V2 summaries if wanted).
