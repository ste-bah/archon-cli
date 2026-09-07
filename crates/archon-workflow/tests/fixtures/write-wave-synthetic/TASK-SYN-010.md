

```yaml
task_id: TASK-SYN-010
title: Create src/alpha.txt seed artifact
complexity: low
status: ready
depends_on: []
blocks: [TASK-SYN-020]
implements: [REQ-SYN-001]
required_env_keys: []
required_tools: []
deliverable_contracts: [{kind: alpha_text, artifact_path: src/alpha.txt, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}]
```

## Purpose

Implement REQ-SYN-001: create `src/alpha.txt` containing the nonempty line `alpha ready`. This is the first task in the frozen chain and produces the seed artifact consumed later by TASK-SYN-020.

Authoring audit marker for this body: SYNTHETIC-CANDIDATE-CANARY-4D8E62B1. Implementation output files must not include this marker or the source PRD marker.

## Scope And Write Boundaries

- May write only `src/alpha.txt` (create the `src/` directory if needed).
- Must not write `.archon/proof/synthetic-observer-target.json` or anything under `.archon/`; that artifact is outside task write ownership and is not created by either task.
- Must not touch `src/beta.json` (owned by TASK-SYN-020).

## Implementation Spec

1. Create `src/alpha.txt` whose contents are exactly one nonempty line:
   ```
   alpha ready
   ```
2. The file must contain no other lines and must not contain any audit/canary marker text.
3. This satisfies the deliverable contract `kind: alpha_text` at `artifact_path: src/alpha.txt` (min_instances: 1).

## Focused Tests

Portable command that reads the exact output and does not invoke Cargo:

```
grep -qx 'alpha ready' src/alpha.txt && test "$(wc -l < src/alpha.txt | tr -d ' ')" -ge 1
```

## Dependency Notes

- `depends_on: []` — this task has no prerequisites and runs first.
- `blocks: [TASK-SYN-020]` — TASK-SYN-020 consumes `src/alpha.txt` and may not start until this task's deliverable exists.