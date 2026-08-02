# Overnight execution plan

Autonomous run. No blocking questions — every open decision is resolved here with its
evidence, and revised in place if execution proves it wrong.

## Hard constraint: disk

`F:` is 466 GB. It has been exhausted twice this session. Rules, enforced by me before every
dispatch:

1. **Maximum two concurrent agent worktrees.** Each costs 6–40 GB once it builds.
2. **Check free space before every dispatch. Below 80 GB, reclaim first, do not dispatch.**
3. **Remove a worktree the moment its branch merges** — not at the end of a phase.
4. `CARGO_INCREMENTAL=0` in every agent brief. The incremental cache reached 72 GB unattended.
5. **Focused tests only** (the user's own NFR-004). `cargo test --workspace` is what exhausted
   the disk both times.
6. `cargo clean` on the main checkout between phases — `target/` reached 206 GB.

## Decisions taken without asking

**D1 — `include!` conversion recipe.** `include!` is unhygienic and its path handling is
platform-specific ([std docs](https://doc.rust-lang.org/std/macro.include.html)); its legitimate
uses are documentation and `build.rs` artifacts, neither of which applies here. Recipe:

```rust
// parent, was:  include!("foo.rs");
#[path = "foo.rs"] mod foo;
use foo::*;              // only if the parent uses items foo defines
```
```rust
// foo.rs gains, as its first line:
use super::*;            // restores what splicing gave for free
```

`#[path] mod` is a real module: own scope, `rustfmt` follows it, CI's fmt gate sees it. Plain
`mod` is preferred where the file can move into a directory. Every failure mode here is a
compile error, so the compiler is the safety net.

**D2 — move and reformat are separate commits.** Converting exposes formatting drift CI has
never seen, because `cargo fmt --check` does not follow `include!`. One commit for the
mechanical move, one for the reformat, so neither is unreviewable.

**D3 — deliverable-contract template placeholders bind through the existing instance fields.**
8 of 17 real tasks declare paths like `datasets/<dataset-id>/<version>/metadata.json`.
`resolve_contract_path` joins them verbatim, so the gate can never pass — this is the user's own
prior-run finding F4, still live. `WorkflowV2DeliverableContract` already carries
`instance_source_path`, `instance_source_records_field`, `instance_artifact_field` and
`min_instances`. That machinery exists precisely for one-contract-many-instances. Templated paths
bind through it; a templated path with no instance binding fails closed with the unexpanded
token named, rather than being checked literally or silently passed.

**D4 — RESOLVED: `/workflow` stays. There is no consolidation to do, because there is no second
execution generation.** `src/command/workflow.rs` is the CLI surface for the live runtime, not a
rival to it. Evidence from `run_action`:

- `Run` / `RunSpec` / `RunTemplate` / `Resume` / `Continue` all return an explicit error:
  *"legacy deterministic workflow execution was removed by the workflow runtime rescue; workflows
  run through the live V2 runtime."* Legacy execution is already gone.
- The file imports `run_live_cli_action`, `should_spawn_live` and `spawn_live_workflow` from
  `workflow_live` — it routes **to** the live runtime.
- `Status` / `Repair` / `Pause` / `Cancel` / `Approve*` / `List` are lifecycle and inspection,
  shared by both paths.
- `Plan { task }` is the only `HeuristicWorkflowPlanner` use, and it merely prints a YAML scaffold
  (`planner.plan(&task)?.to_yaml()?`). It executes nothing.

So the "three generations" framing was wrong in all three parts: v3 is a JavaScript dialect layer
executed by the v2 host, v1 execution was already removed, and `workflow.rs` is the command
surface. **The only real structural debt is the 150 `include!` splices** — Phase 1.

Phase 3 therefore shrinks to deleting `DeterministicStageRunner` (0 references) and nothing else.

## Phases

Each phase ends with: focused tests green, `scripts/check-file-sizes.sh` at 0 offenders,
worktrees removed, disk checked.

### Phase 1 — `include!` conversion in `src/command/workflow*`

150 splices against 111 plain `mod` — more textual pastes than modules, the worst concentration
in the repo. This is the blocker behind everything else: it is why two agents had to fall back to
`#[path]` against instruction, why 17 files are invisible to CI's fmt gate, and why nobody had run
the pipeline end to end until today (`workflow_live_v2_script` is the only scope from which the
planner, task universe, scheduler and review-item builder are all reachable).

Batched, largest cluster first. Behaviour-neutral: no logic edits, visibility widened only as far
as the new boundary requires (`pub(super)` over `pub(crate)` over `pub`).

### Phase 2 — execution ownership

`src/command/workflow*` holds 36.6k LOC of execution logic outside the runtime crate. Phase 1 is
the prerequisite: code cannot be relocated while its scope is defined by splicing. Move execution
into `archon-workflow`, leaving the command layer as a thin CLI surface.

### Phase 3 — v1 consolidation

Delete `DeterministicStageRunner` (0 references). Resolve D4. `WorkflowSpec`, `WorkflowStageRunner`
and `LifecycleController` stay — the live path uses all three.

### Phase 4 — pipeline defects found by the dry run

F4 template binding (D3); contract path collisions (TDL-040/050/060/070 all declare the same
manifest path, all wave 4, all `worktree` write mode); the cycle diagnostic naming no file;
`status:` parsed but never consulted by layering.

### Phase 5 — M4 completion and final verification

Whatever the lint agent leaves. Then one full-workspace run — the only one — with the disk
cleaned first.

### Phase 6 — requirement traceability and satisfaction proof

The PRD half of the original graph-engineering idea, made executable. Motivated by a real failure:
`TASK-TDL-001`'s prior-run finding F1 — *"Accepted verification treats 170 unique IDs as satisfying
normative requirement mapping… artifact sample has repeated generic evidence for REQ-DL-001..004."*
An LLM wrote a gap report claiming 93 requirements were mapped, padded with reused evidence, and a
reviewer caught it. A graph with anchored edges makes that structurally impossible: an edge either
names a `file:line` or it does not exist.

**D5 — `implements:` is an explicit task field, not inferred.** Task YAML gains
`implements: [REQ-DL-020, REQ-DL-021]`. This restores a regression — the older XML task format
carried `<implements><requirement_ref>` and the YAML format dropped it. Today only 3 of the PRD's
93 requirement IDs are cited in any task file, so there is no binding from a requirement to the
verifier meant to prove it. Inference was considered and rejected: F1 is direct evidence that
inferred mapping gets padded, and an explicit field enables two checks inference cannot — every
cited ID must exist in the PRD, and every requirement must be claimed by at least one task
(an unclaimed requirement is a decomposition gap, reported not invented).

**The proof ladder.** Every requirement→code edge carries a level. An unproven edge never counts,
and never satisfies a promotion gate — the same rule REQ-BT-003 already applies to diagnostic
overrides.

| Level | Meaning | Mechanism |
| --- | --- | --- |
| `Candidate` | code is *about* the requirement | `archon-leann` symbol-level search: `file_path`, `line_start`/`line_end`, `file_hash` |
| `Exercised` | a named verifier ran and touched the anchor | verifier `commands_run` evidence + `TraceKind::FileRead` from the ambient trace |
| `Falsifiable` | breaking the anchor breaks the verifier | requirement-scoped mutation, `error`-severity requirements only (PRD §21) |

`Exercised` is the level that kills F1: one command's trace cannot touch four unrelated anchors, so
generic evidence repeated across `REQ-DL-001..004` cannot promote. `file_hash` makes an anchor
invalidatable — when the file changes the edge is known-stale rather than silently wrong.

**Wiring, not new subsystems.** Extract the 93 IDs by regex (they are regular and mostly on their
own bullet line, no LLM needed) into `EntityRecord`s; anchor each via `search_with_filter` scoped by
the task's declared paths; store as `RelationRecord`s; `scan_claims` already detects contradictions
between them.

**Two constraints, both real.** `archon-leann`'s `replace_file_with_cancel`/`remove_file` hold the
Cozo write lock across an entire `multi_transaction` — the longest critical section in the tree — so
indexing must run out of band, never mid-workflow. And `Search::new` needs an `EmbeddingProvider`, so
indexing the repository is a genuine one-off cost with ~93 queries per PRD after it.

`FileRead` is file-granular. Line-granular proof needs coverage (`cargo-llvm-cov`); there is already
a `coverage-analyzer` agent under `.archon/agents/coding-pipeline/`. Treat that as the upgrade path,
not the first cut.

### Phase 7 — SONA-driven parameter tuning

M5 piece 2, revised: `crates/archon-pipeline/src/learning/sona/` already implements the weight
learning a corpus query would have approximated — `get_weight(route, pattern_id)`, `provide_feedback`,
`check_drift`, `save_checkpoint`/`rollback`. Wire it to the four `GeneratedWorkflowConfig`
parameters, keyed by task class. Recorded on [issue #112](https://github.com/ste-bah/archon-cli/issues/112).

**SONA must not touch Phase 6.** It produces a weight; satisfaction is binary and evidence-anchored.
Letting the learning subsystem answer the verification question reproduces F1 with better maths
behind it.

### Phase 8 — motif selection, reframed as structural variation

[Issue #112](https://github.com/ste-bah/archon-cli/issues/112) was filed as deferred on the grounds
that there is no motif library to select among, and that an alternative topology would have to
reproduce the 62-entry plan's earned failure handling (`blocked-*` stages, repair loops, noop-proof
verification, evidence reconciliation, dependency-deadlock detection, final zero-gap audit).

That reasoning holds for *rival plans*. It does not hold for *variations within the proven plan*, and
three things landed since that make the variation form tractable:

1. **A second shape now exists and is measured.** Moving `adversarial-review` from a terminal
   `REDUCE` to a per-task `PARALLEL` stage was itself a motif change, and it produced a measurable
   improvement — attribution became structural, and five findings that had been carried by hand for
   months replay correctly.
2. **The lint suite can score a topology.** Diamond conformance, verifier diversity, fake edges and
   coupled fan-outs are computable on any `TaskGraph`, so a variation can be evaluated before it runs
   and audited after.
3. **SONA supplies the selection machinery**, including drift detection and rollback — see Phase 7.

**So the feasible form is structural knobs on one well-tested topology, not a library of rival
plans.** Every variation inherits the existing failure handling by construction, which was the whole
objection. Candidate knobs, each already a real decision the plan makes:

- review granularity — per task, per wave, or terminal
- verifier count and diversity per task
- repair and investigation iteration budgets (the `GeneratedWorkflowConfig` four)
- fan-out width against the declared dependency graph

Sequenced after Phase 7, because it needs SONA wired and a corpus with real runs in it. A knob with
no outcome evidence for a task class holds its current default rather than being explored
speculatively — the same fail-closed rule the rest of this document applies.

## Out of scope, recorded

M5 motif selection is [issue #112](https://github.com/ste-bah/archon-cli/issues/112): it needs a
motif library before there is anything to select among, and the 62-entry plan encodes earned
failure handling that an alternative motif would have to match. M5's parameter tuning needs real
runs before the corpus can say anything.
