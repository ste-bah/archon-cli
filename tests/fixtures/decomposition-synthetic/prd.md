# PRD-SYNTHETIC

## Objective

Build two generic scratch artifacts in a clean project. The decomposition must produce exactly two canonical tasks with dependency order `TASK-SYN-010` then `TASK-SYN-020`.

## Requirements

| ID | Requirement |
|---|---|
| REQ-SYN-001 | `TASK-SYN-010` creates `src/alpha.txt` containing the nonempty line `alpha ready`. |
| REQ-SYN-002 | `TASK-SYN-020` depends on `TASK-SYN-010` and creates `src/beta.json` with JSON object field `ready` equal to true. |

## Acceptance Criteria

| ID | Criterion |
|---|---|
| AC-SYN-001 | The project artifact `.archon/proof/synthetic-observer-target.json` exists, is JSON, and has boolean field `ready` equal to true. Freeze this criterion as a commandless floor with `kind="synthetic_observer_target"`, `artifact_path=".archon/proof/synthetic-observer-target.json"`, `artifact_format="json"`, `required_true_fields=["ready"]`, every other floor field at its serde default, and no `typed_verifier_command`. This artifact is outside task write ownership and the tasks must not create it. |

## Authoring audit markers

- The source PRD carries prompt marker `SYNTHETIC-PROMPT-CANARY-7F3A91C2`.
- Every generated canonical TASK body must include the literal prose marker `SYNTHETIC-CANDIDATE-CANARY-4D8E62B1`; implementation output files must not include either marker.

## Task boundaries

- `TASK-SYN-010` may write only `src/alpha.txt`.
- `TASK-SYN-020` may write only `src/beta.json`.
- Both tasks use portable focused tests that read their exact output and do not invoke Cargo.
- No task may write `.archon/proof/synthetic-observer-target.json`.
