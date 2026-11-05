# Workflow V2 Boundary

This module is the dynamic workflow runtime boundary.

The V2 path must not treat legacy generated YAML stages or markdown artifacts as the workflow control plane. Generated workflow harnesses, typed host calls, typed host-call results, and durable result caching belong here.

Initial implementation order:

1. typed result contracts
2. harness validator and host API
3. runtime/result store/resume
4. scheduler and parallel fanout
5. live agent adapter
6. decomposed PRD intake
7. safe write modes
8. verification/review/remediation loops
9. final report
10. CLI/TUI/web integration

Release builds remain blocked until TASK-CWF-140 passes.

## Call semantics

Calls can declare `options.metadata` independently of their ids:
`{ "kind": "focused_verification", "task_scoped": true, "sequence": 7 }`.
`kind` selects the evidence and verification contract; `task_scoped` permits
reuse across ordinal changes within the same kind; optional `sequence` declares
restart order. `source_graph: false` explicitly opts a call out of canonical wave
graph construction (used by direct task writes). Explicit dependencies and task intersections also propagate invalidation.
Kinds are `other`, `noop_proof`, `implementation`, `remediation`,
`review_remediation`, `focused_verification`, and `review_verification`.
Explicit metadata takes precedence over item kinds and historical id conventions.
The primitive API supplies metadata for its work and verification calls.
Unannotated recorded calls retain legacy interpretation for resume compatibility.
Use `binding` (or `agent`) to select an exact agent identity; `role` declares
its role. Tool grants are explicit; task prose never grants command access.
