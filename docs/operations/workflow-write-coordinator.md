# Workflow Write Coordinator (PRD-012)

The Write Coordinator makes parallel implementation fan-out safe by default. An
implementation fan-out stage normally pins its width to 1 so two agents never
race on the same files. The coordinator lifts that pin: it runs each item in an
isolated `git worktree`, schedules non-conflicting items into the same wave, and
applies validated patches back to the canonical repository under a cross-process
write lock. When coordination is unavailable it falls back to the existing
serial path, so behaviour is never less safe than before.

## When it activates

The coordinated path runs only when all three hold:

| `enabled` | canonical is Git | runner supports boundary | Result |
|---|---|---|---|
| `true`  | yes | yes | Coordinated parallel |
| `true`  | yes | no  | Serial fallback (`boundary_unavailable`) |
| `true`  | no  | n/a | Serial fallback (`non_git_root`) |
| `false` | n/a | n/a | Serial fallback (`feature_disabled`) |

- **Config flag** — `[workflow.write_coordinator] enabled` (default `true`). See
  [config reference](../reference/config.md#workflowwrite_coordinator-prd-012--parallel-implementation-fanout).
- **Git canonical root** — the workflow spec's `target_repository_root` must be a
  Git repository (a `.git` directory or file is present).
- **Boundary-supported runner** — the tool runner must report
  `supports_workspace_boundary() == true`. The live `PipelineWorkflowRunner`
  does; the deterministic and most test runners do not (and fall back to serial).

A non-implementation fan-out stage is never touched by the coordinator.

## Required per-item target declaration

When `fail_on_undeclared_write = true` (default), every item of an
implementation fan-out stage with inline `input.items` MUST declare per-item
`target_files` (or `expected_target_files`). A stage whose items omit both is
rejected before launch with
`WorkflowError::ImplementationFanoutMissingPerItemTargets`. Set
`fail_on_undeclared_write = false` to allow undeclared items; they then collapse
to mutually conflicting and are scheduled serially.

## Isolated workspace layout

Coordinator artifacts live under the workflow store's run directory
(`<project>/.archon/workflows/<run-id>/`):

```text
.archon/workflows/<run-id>/
  wc/worktrees/<stage-id>/<item-id>/          # per-item git worktree
  write-coordination/stages/<stage-id>/
    manifests/<item-id>.json                  # PatchManifest (status + hashes)
    patches/<item-id>.patch                   # captured patch bytes
    apply/<wave-id>.json                      # ApplyRecord per wave
    tests/<wave-id>.json                      # wave verify result
```

## Dirty canonical handling

For each item the coordinator captures the canonical dirty state and reproduces
it in the isolated worktree:

- Tracked changes are replayed via a `git diff --binary HEAD` overlay applied
  with `git apply --3way`.
- Declared untracked target files are copied in (bytes + mode); files that are
  neither declared targets nor verify inputs are never read into memory, so
  secrets such as `.env` cannot leak into the workspace.
- A detached baseline commit seals the reproduced state so later diffs are
  well-defined.

Untracked declared targets larger than `max_file_bytes` are rejected at capture
time with `IsolationError::FileTooLarge` before their bytes are loaded.

## Wave scheduling

Items are scheduled into ordered waves:

- Two items conflict when any pair of their resource keys overlaps. Keys are
  `file:<path>`, `dir:<path>` (only for parent directories the item creates), or
  `glob:<path>` (only for explicitly declared globs). A file under a directory
  key conflicts with that directory; a glob conflicts with files it matches.
- Items whose targets came from the shared stage-level fallback are treated as
  mutually conflicting.
- Item-level `depends_on` forces a dependent into a strictly later wave.
- Maximum wave width is the floor of the run, policy, stage, runner
  (`max_concurrency`), and subagent caps — never below 1.

Scheduling is deterministic (topological order with declared order as the
tiebreak), so the same input always produces byte-identical waves.

## Patch validation

Before any patch touches canonical, the coordinator validates it:

| Check | Rejects when |
|---|---|
| VAL-WC-001 declared-only | a changed file is not a declared target |
| VAL-WC-002 in-repo | a changed path escapes the canonical root |
| VAL-WC-003 symlink | a changed file is a symlink resolving outside the root |
| VAL-WC-005 size budget | patch > `max_patch_bytes` or a file > `max_file_bytes` |
| VAL-WC-006 secret scan | an added line matches an anchored credential regex |
| VAL-WC-007 empty patch | the patch is empty and the item did not declare `idempotent_noop` |
| VAL-WC-008 output usable | the agent body self-reports blocked/failed |

VAL-WC-004 (stale-baseline recheck) runs later, at apply time.

## Apply ordering, lock, and verify

All patches in a wave are applied serially in item-id order, inside one
cross-process advisory write lock keyed by a BLAKE3 hash of the canonical root
(`<canonical>/.archon/workflows/write-locks/<hash>.lock`). The lock retries for
up to ~60s before returning `LockTimeout`. Before each apply the coordinator
re-hashes the declared targets the item intends to change; a drift yields
`StaleBaseline` and the item is skipped. A failed `git apply --3way` is cleaned
up with `git checkout HEAD -- <paths>` and reported as `PatchApplyConflict`. The
wave's `verify_command` (when present) runs inside the same lock; a non-zero
exit marks the wave's verify result failed with no automatic rollback.

## Resume semantics

On resume, each item's persisted manifest status decides its fate:

- **Applied** / **IdempotentNoop** — skipped (already done).
- **Failed** / **PendingApply** — re-executed via the coordinator.
- **Conflicted** — surfaced to the operator; never auto-retried.
- **NotPersisted** — a new item, run normally.

## Status

`workflow` completion renders a compact six-line block per coordinated stage:

```text
write_coordination: enabled
stage: <stage_id>
wave: <wave_index>/<wave_total>
width: <width>
items: <running> running, <failed> failed, <accepted> accepted
apply: <apply_state>
```

A fallback run renders a single line, e.g.
`write_coordination: serial_fallback (non_git_root)`.

## Error reference (§19)

| Code | Concrete error | Operator hint |
|---|---|---|
| WC-ERR-MISSING-TARGETS | `ImplementationFanoutMissingPerItemTargets` / `WritePlanError::MissingTargets` | Declare per-item `target_files` in the spec. |
| WC-ERR-INVALID-TARGET-PATH | `WritePlanError::InvalidTargetPath` | Use a relative path under the canonical root; no `..` or empty segments. |
| WC-ERR-BOUNDARY-UNAVAILABLE | `SerialFallbackReason::BoundaryUnavailable` | The runner cannot confine writes; serial fallback is automatic. |
| WC-ERR-BASELINE-REPRODUCTION | `IsolationError::ApplyFailed` / `BaselineCommitFailed` | The dirty canonical state could not be reproduced; commit or stash conflicting changes. |
| WC-ERR-CANONICAL-MUTATION | `IsolationError::CanonicalMutation` | An agent wrote directly to canonical; verify the file tool honours `target_repository_root`. |
| WC-ERR-UNDECLARED-WRITE | `PatchError::UndeclaredWrite` | The patch touched a path outside declared targets; expand `target_files` or split the item. |
| WC-ERR-STALE-BASELINE | `ApplyError::StaleBaseline` | A declared target changed under coordination; restart the stage. |
| WC-ERR-PATCH-APPLY-CONFLICT | `ApplyError::PatchApplyConflict` | 3-way apply failed; canonical is auto-cleaned. Resolve manually or split the item. |
| WC-ERR-WAVE-VERIFY | non-zero `VerifyResult.exit` | Wave verify failed after apply; inspect `tests/<wave>.json`. |
| WC-ERR-SECRET-DETECTED | `PatchError::SecretDetected` | The patch contained a credential-like string; remove it and retry. |
| WC-ERR-FILE-TOO-LARGE | `PatchError::FileTooLarge` / `IsolationError::FileTooLarge` | A file exceeds `max_file_bytes` (a runtime byte budget); raise the limit or split the file. |
| WC-ERR-PATCH-TOO-LARGE | `PatchError::PatchTooLarge` | The patch exceeds `max_patch_bytes`; raise the limit or split the item. |
| WC-ERR-CONFLICT-GRAPH-VIOLATION | `ApplyError::ConflictGraphViolation` | The scheduler produced overlapping targets in one wave; this should not happen — file a bug with the `ApplyRecord`. |

## Known limitations

- No auto-merge of semantic conflicts within the same file (NG-WC-002).
- No provider-specific isolation beyond the `StageRunRequest.input`
  `target_repository_root` rewrite (NG-WC-004).
- Applying a patch back to a **dirty tracked** declared target can fail
  `git apply --3way` with "does not match index" (the 3-way base is the index,
  not the dirty working tree). The dirty state is still reproduced correctly in
  the isolated worktree; commit or stash the conflicting tracked change before
  the run to apply cleanly.

## See also

- [Configuration](../reference/config.md) — `[workflow.write_coordinator]` keys
- [Troubleshooting](troubleshooting.md) — Write Coordinator error recipes
- [Data locations](data-locations.md) — where run artifacts live
