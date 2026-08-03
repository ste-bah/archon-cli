# Topology

Archon runs work as a graph whether or not anyone drew one. A workflow spec
declares its stages; a team run decomposes into subtasks; an ordinary coding
turn spawns subagents that write files. Those three produce the same shape, and
the topology subsystem is what gives that shape one representation, one
recording format, and one place where structural invariants are enforced.

Three layers, deliberately separated by what they are allowed to cost:

| Layer | Cost | Where it runs |
|---|---|---|
| `TaskGraph` IR | pure, in-memory | `archon-topology` |
| Ambient trace | one file append per event | hot path of every tool call |
| Guardrail admission | `DashMap` lookups, synchronous | inline, before each tool executes |
| Batched fold | a Cozo write | once, after a graph completes |

The ordering matters: nothing on the hot path may touch a database, and the
fold that does touch one never runs while work is in flight.

## The `TaskGraph` IR

`TaskGraph` (`crates/archon-topology/src/ir.rs:240`) is the common
representation:

```rust
pub struct TaskGraph {
    pub id: String,
    pub origin: GraphOrigin,
    pub nodes: Vec<TaskNode>,
    pub budget: GraphBudget,
}
```

`GraphOrigin` records which of the three producers built it — `Workflow`,
`Team`, or `Session` (`ir.rs:10`).

A `TaskNode` (`ir.rs:147`) carries its `id`, a `role`, its `depends_on` list,
what it `consumes`, what it `writes`, what it `reads`, a `permission` class, an
optional `agent`, and an optional `fanout` spec. `NodeRole` (`ir.rs:33`) is
`Plan`, `Work`, `Verify`, `Reduce`, `Gate(GateKind)`, or `Tool`.
`PermissionClass` (`ir.rs:100`) is `Safe`, `Risky`, or `Irreversible`, and
defaults to `Safe`.

**There is no edge type.** Edges exist only as strings in
`TaskNode::depends_on`. The graph is materialised on demand as a petgraph
`DiGraph<usize, ()>` with unit edge weights (`crates/archon-topology/src/index.rs:21`).
Edge *classification* — the `Dataflow` / `OrderingOnly` / `Unsupported` split
that [`archon workflow lint`](../reference/workflow-lint.md) reports — is an
analysis output computed from the node declarations, not a property stored in
the IR (`crates/archon-topology/src/analysis/edge_support.rs:95`).

### The two lowerings

Neither of them lives in `archon-topology`. A lowering reads a type the topology
crate does not own, so siting it there would invert the dependency edge — and in
the `WorkflowSpec` case it closed a cycle
(`archon-core → archon-workflow → archon-topology → archon-core`) that Cargo
rejects outright.

**`WorkflowSpec` → `TaskGraph`** is `lower_workflow_spec`
(`crates/archon-workflow/src/lower_workflow.rs:39`). It is infallible. Stage
kinds map onto roles — `QualityGate` becomes `Verify`, `HumanGate` becomes
`Gate(Human)`, `Checkpoint` becomes `Gate(Checkpoint)`, and `Agent`,
`Implementation` and `Fanout` all become `Work` (`lower_workflow.rs:105`).
`writes` comes from the stage's `expected_target_files`; `consumes` is recovered
only from a fanout `${producer.accessor}` reference (`lower_workflow.rs:125`,
which now calls `spec::parse_foreach_accessor` directly rather than carrying a
byte-compatible copy of it); `reads` is always empty (`lower_workflow.rs:85`).
Permission classes are resolved per stage, then from a blanket `default`/`*` key
(`lower_workflow.rs:158`).

**`Vec<Subtask>` → `TaskGraph`** is `lower_subtasks`, and it lives in
`crates/archon-core/src/orchestrator/topology.rs:55`, because `Subtask` is an
`archon-core` type and the topology crate does not depend on `archon-core`.
Every node is `Work`; only `depends_on` and `agent` are populated. `consumes`,
`writes` and `reads` stay empty, and `permission` stays `Safe` for every node.

That asymmetry is the single most important fact about the IR in practice:
**empty means unknown, not empty.** A team-lowered graph declares no dataflow,
so the dataflow analyses have nothing to conclude and say so rather than
reporting a clean result. The same rule is why `archon workflow lint` prints
"no node declares what it consumes" instead of "no findings".

### Validation

Lowering never validates. Validation happens when the graph is indexed, in
`GraphIndex::build` (`index.rs:38`), and there are exactly three ways a graph
can be rejected (`crates/archon-topology/src/error.rs:14`):

- `DuplicateNode { id }` — two nodes share an id, so `depends_on` is ambiguous.
- `UnknownDependency { node, dependency }` — a dependency names a node that is
  not in the graph.
- `Cycle` — `depends_on` has no topological order.

`TaskGraph::validate()` and `TaskGraph::waves()` are the entry points
(`crates/archon-topology/src/analysis/waves.rs:19`).

`GraphBudget` (`ir.rs:124`) carries `max_parallelism`, `max_agents` and
`max_rounds`, defaulting to 8 / 200 / 1 (`ir.rs:134`) to agree with the
`WorkflowSpec` defaults.

## The ambient trace

Every graph gets a directory under `<project>/.archon/topology/<graph-id>/`
containing `graph.json` (the declared graph, when there was one) and
`trace.jsonl` (what actually happened). The constants are in
`crates/archon-topology/src/trace/paths.rs:16`. Graph ids are sanitised to
`[A-Za-z0-9._-]` before being used as a directory name (`paths.rs:150`).

One record is one JSON object on one line
(`crates/archon-topology/src/trace/record.rs:52`). `ts`, `graph_id` and `kind`
are always present; `node_id`, `agent`, `tool`, `permission`, `blocked`,
`error`, `writes`, `reads`, `duration_ms`, `attempt` and `detail` appear when
they apply. `TraceKind` (`record.rs:23`) has ten written variants —
`GraphDeclared`, `NodeStarted`, `NodeFinished`, `AgentSpawned`, `ToolAttempt`,
`FileWritten`, `FileRead`, `GatePassed`, `Verification`, `Retry` — plus an
`Unknown` catch-all that is only ever read, never written.

### The hot path never touches a database

This is a design constraint, not an aspiration, and it is enforced three ways:

1. **By the dependency graph.** `archon-topology` depends on `petgraph`,
   `serde`, `serde_json`, `dashmap` and `archon-write-plan` — a leaf crate with
   no `archon-*` dependencies of its own, holding the resource-key overlap table
   that live admission and the write coordinator must both answer from
   (`crates/archon-topology/Cargo.toml`). There is no Cozo dependency to call,
   and since Wave E no `archon-workflow` dependency either.
2. **By the write path.** `TraceWriter::append`
   (`crates/archon-topology/src/trace/writer.rs:64`) opens the file with
   `create(true).append(true)` and issues one `write_all`. Its caller
   `AmbientTrace::record` (`src/command/topology_trace.rs:136`) swallows every
   error — a trace failure must not fail the tool call it was observing.
3. **By test.** `a_full_session_performs_no_database_access`
   (`src/command/topology_fold/tests/hot_path.rs:24`) arms
   `archon_cozo::poison_guarded_scripts()` and drives a whole session through
   the taps. A companion test asserts the poison actually fires on a guarded
   write, so the check cannot pass vacuously.

Records are capped at 16 KB (`writer.rs:20`). Over the cap, `encode_record`
sheds `detail`, then the target lists, then falls back to a minimal record
(`writer.rs:76`). A record is truncated, never dropped.

### The fold into `.archon/topology.db`

When a graph completes, its trace is folded once into a Cozo-over-SQLite
database at `<project>/.archon/topology.db`
(`src/command/topology_fold/schema.rs:14`).

The trigger is graph completion, and there are two call sites: a team run
(`src/command/team.rs:86`) and a workflow run
(`src/command/workflow_live_v2_run_fold.rs:40`). Both hand the fold to
`tokio::task::spawn_blocking`, so it is off the hot path but synchronous within
its own task — `fold_graph` (`src/command/topology_fold.rs:137`) is blocking
and must not be called from async directly.

The fold reads `trace.jsonl` and `graph.json`, derives a summary, ensures the
schema, writes the rows, and writes an `ingested` marker file **last**
(`topology_fold.rs:146`). A directory already carrying that marker is skipped,
and the row writes are `:put` upserts keyed on `graph_id`/`node_id`, so the
fold is idempotent from both directions.

`derive` (`src/command/topology_fold/derive.rs:39`) is pure and does no I/O. It
reconstructs a skeleton graph when none was declared, then computes the critical
path span, peak observed parallelism, write conflicts, observed retries,
per-node outcomes and durations, and a task hash of the goal text. The result
lands in three relations — `topology_graph`, `topology_node`,
`topology_outcome` — plus a `by_task_hash` index, written in a single script
(`src/command/topology_fold/rows.rs:16`).

## Guardrail admission

Three structural invariants are checked before a tool runs, synchronously, in
the same call stack that is about to execute it.

The hook is `ToolRunAdmissionCallback`
(`crates/archon-tools/src/tool.rs:62`):

```rust
pub type ToolRunAdmissionCallback =
    Arc<dyn Fn(ToolRunAdmissionRequest) -> ToolRunAdmission + Send + Sync>;
```

It returns a value, not a future. It is invoked in `execute_tool_attempt`
(`crates/archon-core/src/tool_run_admission.rs:19`) before `tool.execute(...)`,
and a `Blocked` verdict returns early: the tool does not run, and the model
receives `ToolResult::error("ToolRun blocked: {reason}")`. That is a hard block
on that attempt — not a warning, and not an error that aborts the turn.

Everything reachable from the callback is `DashMap` lookups and string work.
The expensive structural work — dominator computation, transitive reachability —
happens once when the graph is declared (`crates/archon-topology/src/live.rs:252`),
not per call. `a_full_admission_pass_performs_no_database_access`
(`src/command/topology_admission/tests/hot_path.rs:25`) holds that line.

Admission is skipped entirely for `Safe` tool calls
(`tool_run_admission.rs:19`).

### Agent cap

`state.agents_spawned() >= state.budget().max_agents`
(`crates/archon-topology/src/live/agents.rs:48`). The count is a **lifetime**
total for the session and is never decremented; finishing an agent decrements
only the live count (`crates/archon-topology/src/live/state.rs:215`). The limit
is a declared graph's `GraphBudget::max_agents` when there is one, otherwise the
configured `max_agents`, default 200. Budget is consumed only on admission, so a
blocked spawn does not spend one.

### Single writer

For each path a node declares it will write, admission asks whether a
*different* node holds a conflicting claim
(`crates/archon-topology/src/live/writes.rs:49`). A claim conflicts only when
all four hold (`live/state.rs:223`): it belongs to another node, that node is
still live, neither node is reachable from the other, and the two keys overlap
under the workflow write coordinator's rules.

There is no numeric limit here — it is a mutual-exclusion check. An empty write
list means unknown and is not checked at all (`writes.rs:54`). The one place it
does not fail open is a malformed glob, which is treated as conflicting
(`writes.rs:27`). Claims are taken only after every path clears.

### Ungated irreversible

Fires only when the action's `PermissionClass` is `Irreversible`
(`crates/archon-topology/src/live/gates.rs:44`). What happens then depends on
the configured enforcement mode:

- `off` — never blocks.
- `where_declared` (default) — blocks only in a session whose structure
  declares at least one gate.
- `always` — evaluates regardless.

For a node the graph knows, the action is admitted when some gate dominating
that node has passed (`gates.rs:72`). For an unknown node — an ordinary turn,
which declares nothing — it is admitted when any gate has passed at all
(`gates.rs:64`).

**A `Checkpoint` gate never counts.** Nothing in the tree marks one passed, and
there are tripwires in both the lowering and the live layer to keep it that way
(`crates/archon-workflow/src/lower_workflow.rs:92`,
`crates/archon-topology/src/live.rs:359`).

The default of `where_declared` exists because `always` would block every
irreversible action in every session that never declared a graph, which is every
ordinary coding turn.

### Where it fails open

Deliberately, and in these cases only:

- no tracker installed (`src/command/topology_admission.rs:64`);
- an untracked session (`live.rs:274`);
- a `Safe` permission level (`tool_run_admission.rs:19`);
- a graph that fails structural validation — it is ignored rather than treated
  as a reason to block (`live.rs:256`);
- more than 256 concurrently tracked sessions, where the *new* session is
  dropped (`live.rs:88`).

### Two limits worth knowing

**Node attribution is coarse.** Every tool call is attributed to the turn root
unless the tool is a spawn. Two subagents writing the same file therefore look
like the root writing it twice — a self-conflict, which is admitted. This
under-reports conflicts by design; it is documented at
`src/command/topology_admission.rs:31` and `src/command/topology_trace.rs:41`.

**Spawn detection is a hardcoded name list** — `Agent`, `Task`, `TaskCreate` —
duplicated in `src/command/topology_admission/translate.rs:26` and
`src/command/topology_trace/tool_tap.rs:18`. A spawning tool outside that list
is not counted against the agent cap.

## Configuration

All five keys live under `[topology]` and are documented in the
[configuration reference](../reference/config.md#topology-guardrail-admission):
`admission_enabled`, `agent_cap`, `single_writer`, `ungated_irreversible` and
`max_agents`. There are no environment variables for any of them, and no CLI
flag or slash command toggles them at runtime. Setting `admission_enabled =
false` installs no tracker at all.

## Related

- [`archon workflow lint`](../reference/workflow-lint.md) — the advisory
  analyses over a lowered graph.
- [`archon requirements trace`](../reference/requirements-trace.md) — consumes
  the `FileRead` records this trace emits.
- [Dynamic workflows](dynamic-workflows.md) — the workflow engine that produces
  `WorkflowSpec`.
