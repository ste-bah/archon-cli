# Workflow Runtime Rescue — Phase 3 Report: Declared Artifact Contracts

Branch: `rescue/workflow-runtime-v3`
Result: **`canary_wf_afae6bee_regression` is GREEN** — one phase ahead of the
handover's schedule. The reconstructed wf-afae6bee run now completes
end-to-end with an accepted final report: declared artifact → instructed
agent → artifact written → verification accepted on the first pass → review →
final gates → accepted.

## The contract (one source of truth)

An artifact requirement is validated on return only if it was explicitly
declared and handed to the agent:

- `WorkflowV2ArtifactRequirement { path, kind }` + `required_artifacts` on
  `WorkflowV2HostOptions` (parsed from `requiredArtifacts`/
  `required_artifacts` in script options).
- Task packs declare artifacts (`## Artifact Requirements` sections /
  `artifact_requirements:` fields in TASK-*.md), parsed into the task
  universe; the lifecycle unions them into every write-capable item at fanout
  time — the wf-afae6bee fix: the declaration always reaches the implementing
  agent.
- One extraction function (`declared_project_artifact_entries`) drives BOTH
  the prompt's resolved-path section (now with explicit "write these and list
  them in artifacts[]" contract wording) AND the on-return validation, so a
  path can only be validated if the agent was instructed to produce it.
- A missing declared artifact is a **failed result value**
  (`data.missing_required_artifacts`), never a run-level block.

## Root causes fixed (each found via the canary, each a producer/consumer mismatch)

1. **Role-name write capability**: `is_write_capable()` treated role "coder"
   as write-capable, so read-only verification agents (coder tier, told "Do
   not modify files") were rejected for not changing files — the actual
   verification death spiral of wf-afae6bee. Write capability is now declared
   via `write_mode` only.
2. **CWD-relative artifact checks**: `requireArtifact` and the final report's
   artifact-existence guard resolved relative paths against the process cwd;
   both now resolve against the project root that owns the run's `.archon`.
3. **Completion ledger starvation**: the write-fanout outcome view dropped
   `completion_evidence`, so implementation evidence never reached the durable
   ledger and the final acceptance gate could never accept a live run. The
   view now persists it.
4. **Final report self-sabotage**: the report builder downgraded to
   NeedsReview because its direct source results (host-produced gates) carry
   no commands; acceptance is restored when the ledger reconciles cleanly and
   supplies the command evidence.

## Deleted (inference and self-heal)

- `required_artifact_contract.rs`, `required_artifact_heal.rs`,
  `required_artifact_repair_guidance.rs`, `required_artifacts.rs` + their
  executor/generated_parts call sites and six test binaries (this also erased
  7 of the 12 pre-existing baseline test failures and all 10 pre-existing
  archon-workflow clippy errors).
- The deep-walk inference in `project_artifact_completion.rs` (replaced by
  declared-only enforcement).
- The artifact self-heal in `agent_adapter.rs`
  (`project_artifact_branch_result` / synthetic results on agent failure) and
  its four rescue tests; agent failures are failures.

## Test status

- Canary: green (the rescue's acceptance test).
- Bin crate: 289 workflow tests green; 1 pre-existing red
  (`generated_v2_read_only_verification_branch_stays_read_only`, Phase 5).
- Crate: 5 pre-existing reds remain (`generated_task_items` ×2,
  `runtime_continuation` ×3 — legacy modules, Phase 4/5 delete list).
- `cargo clippy -p archon-workflow`: clean. FileSizeGuard: green.

## Carried forward to Phase 4/5

- Phase 4 (error-as-value) is now mostly about deleting the terminal latch and
  the remaining Rust-side compensation (`remediation_*`, `quality_gate`
  overlap, `context_output` heuristics) — the canary already proves the happy
  path; Phase 4 must keep it green while making failures script-consumable.
- Behavior note: legacy spec runs no longer get `required_artifact` stages
  injected into their specs (module deleted); the legacy executor path itself
  is Phase 5 demolition scope.
