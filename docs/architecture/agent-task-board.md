# Agent task board

The task board is how agents in one run hand work and findings to each other. A
reviewer that finds a problem raises an item with the file references and what
"done" would mean; an implementer claims it, works it, and moves it along; the
parent can re-scope it or escalate it. The alternative — passing findings as
prose in a subagent's return value — loses everything the parent does not
happen to quote forward, and gives no way for a second agent to take ownership
of a finding without two agents doing the same work.

This document describes the board as built: the four tools agents use, the host
helpers that raise an item on an agent's behalf, the lifecycle an item moves
through, the storage and concurrency design underneath, and the gate that stops a
run being reported complete over an undrained board.

## What an agent does with it

Four tools, offered to every subagent. They are the whole surface an agent
*calls*; the host also raises items on a dispatched agent's behalf, which is
[below](#what-the-host-raises-on-an-agents-behalf).

| Tool | What it does |
|---|---|
| `BoardRaise` | Records a finding. `kind` is `issue` (work that must happen) or `note` (context for whoever next touches the area). `title` and `evidence` are required; `acceptance` says what "done" means. |
| `BoardList` | Lists this run's items, oldest first, optionally filtered by `status`. `open` items are unclaimed and available. |
| `BoardClaim` | Takes ownership of an item by id. Exactly one caller wins a contested claim; the loser is told which agent holds it. |
| `BoardResolve` | Closes an item as `resolved` or `declined`. A reason is required either way. |

**`evidence` is required at the write, not encouraged.** Whoever claims the item
cannot ask the raiser afterwards — that agent has finished. An item without
file references means rediscovering the finding from scratch, which is the
failure the board exists to prevent. The same reasoning makes `reason`
mandatory on `BoardResolve`: an item that disappears without one cannot be
distinguished from an item that was quietly dropped.

`BoardList` sweeps dead claims before it answers, so a `claimed` item in the
listing really does have a live owner. `BoardClaim` sweeps too, for the same
reason — an item held by an agent that has since exited is available, and the
claimant is the party who cares.

### The tools reach every subagent

`ALWAYS_ALLOWED` in `subagent_executor.rs` unions the four into a subagent's
tool set however that set was derived — from an explicit `allowed_tools`, from
the agent definition, or from `DEFAULT_TOOLS`.

This matters because most pipeline agents name their tools explicitly, and a
board absent from precisely the fan-outs it exists to coordinate would be a
board that works only where it is not needed. Withholding it does not restrict
what an agent can **do**; it removes its ability to say what it found. A tool
list is written to bound blast radius, and the four widen none — they are
`Safe`, run-scoped, and mutate nothing outside the board.

`DENYLIST` still wins, so this is an always-*offer* set, not an override of a
deliberate refusal.

## What the host raises on an agent's behalf

Three host-side helpers write to the board without an agent calling a tool.
`raise_delegated_task` and `close_delegated_task` back the `TaskCreate` and
`Agent` spawn tools: the parent's dispatch raises a claimed item so a spawned
agent is visible on the board without being asked to announce itself.

`raise_delegated_branch` does the same for a **workflow stage branch**, and it
exists because the spawn *tools* are not the spawn *paths*. Workflow stages
dispatch through the V2 lifecycle and reach neither tool, so a run that
dispatched seven stages left the board empty for hours while the half of the
system doing the work was invisible (#161). Both V2 dispatch routes are wired —
`PipelineWorkflowRunner::run_stage` and `LiveV2AgentClient::run_agent_request`,
the latter being the one a real decomposed run takes.

Two properties matter to anyone reading the board:

- **A stage branch is a `note`, not an `issue`.** The drain gate counts an
  unresolved issue as undrained, and a branch closes `in_review` by design, so
  raising branches as issues makes every run gate itself — measured, not
  reasoned: it turned the full-lifecycle fixture's `Accepted` into `NeedsReview`
  with `blocked-board-drain`. An issue outlives the run; a note dies with it, and
  a stage branch is the run executing itself.
- **The item id is not the session id.** It is `{session}-{ordinal}-{agent}`, the
  id the subagent adapter mints and registers for liveness. A claim held under an
  id no registry knows would be swept back to `open` by `release_dead_claims`
  while the stage was still running.

The item is owned by an RAII guard that closes `Failed` on drop unless a verdict
was set, so an exit that unwinds still closes its item rather than leaking one
that reads as live work forever. Every board write on this path is soft: none can
fail or delay a stage. Partitioning needed nothing new — `run_id_for_session`
splits a stage session id on the first `-stage-`, yielding the run's own `wf-…`
id.

## The item lifecycle

`status` moves through `open`, `claimed`, `in_review`, `gaps_remain`,
`resolved`, `declined`, `promoted`, `escalated`.

The ordinary path is `open → claimed → resolved`. `gaps_remain` is a reviewer
saying the work is not finished, and sends the item back to `open` or `claimed`
for another attempt. `promoted` means the item outlived the run and became a
tracked issue; `escalated` hands it to the parent; `declined` closes it as work
that should not happen.

`round` counts re-attempts, not transitions. It advances when an item leaves
`gaps_remain` for a working status, because that is the only move in the
lifecycle that means *try again*; every other transition carries the count
forward. Incrementing on each move would duplicate the event history and would
report a straight `open → claimed → resolved` item as three attempts. The rule
is about the *pair* of statuses, so it is computed in Rust and passed in rather
than restated in Datalog as a `%if` per transition.

## Why it is not a `MemoryType`

The board lives in its own `board_items` Cozo relation in the memory graph's
database, not as rows in `memories`. That was not the obvious choice — memories
already have storage, a server, a remote client, and a search path — so the
reasons need to be concrete.

**The memory schema is fixed at twelve columns.** Board state has an id, a run,
a kind, a status, a title, evidence, acceptance criteria, a raiser, a claimant,
and a round counter. There is nowhere to put those except the tag vector.

**The tag vector has a hard ceiling of sixteen non-trend tags, and it is
asserted in Datalog, not in Rust.** `graph/crud_importance.rs:40` asserts that
index 17 of the parsed tag array is null, and line 46 asserts that the retained
(non-`trend:`) tag list is at most sixteen long. An encoding that overflows that
does not degrade — it fails the assertion inside the update script.

**`update_memory` is last-writer-wins with no compare-and-set.** It replaces the
whole tag vector. Two agents claiming the same item would both write a claim tag
and both believe they won, which is precisely the failure the board exists to
prevent.

**A type-only filter is a full relation scan.** A board is polled: by every
agent looking for work, and by the drain gate at every barrier. That cost is
paid constantly and must not grow with the size of the whole store.

**The precedent is discouraging.** `PersonalitySnapshot` took the other route —
serialise JSON into `content`, mark it with a type — and the consequence was
`drop_state_snapshots` filters retrofitted at three separate read paths, because
every reader that had ever assumed `content` was prose now had to be taught
otherwise.

A relation gets real columns, a real index, and a real compare-and-set.

## Schema

```
board_items {
    id: String
    =>
    run_id, kind, status, title, evidence, acceptance, raised_by,
    claimed_by: String?, round: Int, created_at, updated_at
}

::index board_items:by_run {run_id}
```

`run_id` partitions the board, and it is the partition
[the drain gate](#the-drain-gate) is defined over. Every subagent inherits its
parent's, and a workflow stage session id resolves to its run's. `list_board_items_by_run`
goes through `board_items:by_run` rather than filtering a scan, for the polling
reason above.

`kind` is `issue` or `note`, kept apart because the lifecycles differ: an issue
outlives the run that raised it and must resolve, be promoted, or be declined; a
note is context for whoever next touches the area. Conflated, the board fills
with "looked at X, seemed fine" and the drain gate becomes noise.

`status` and `round` are described in [The item lifecycle](#the-item-lifecycle).

**`claimed_by` is nullable rather than an empty-string sentinel**, and this is
load-bearing rather than stylistic. The claim precondition is `is_null(claimed_by)`.
Unclaimed has to be a state the database itself can test, not a convention every
query re-implements — and the row decoder is written to match: a null decodes to
`None`, never to `Some("")`, because `Some("")` would read to the CAS as "held
by nobody in particular" and the item would be unclaimable forever
(`board/rows.rs`, `row_values_to_item`).

`evidence` and `run_id` are rejected at the write if empty. Evidence is rejected
because an item without file references cannot be acted on by whoever picks it
up — they would have to rediscover the finding from scratch, which is the
failure the board exists to prevent, and the write is the only place the rule
can hold: by the time a claimant reads the row, the agent that knew the
references is gone. Creating over an existing id is also refused rather than
overwritten, because `:put` is last-writer-wins and a collision would destroy
another agent's item along with its round history.

## The claim is a compare-and-set

`claim_board_item`, `release_board_claim`, and `set_board_item_status` all go
through one helper, `board_cas` (`board/claim.rs`). It builds a single Cozo
script of this shape:

```
{ ?[id] := *board_items{<guard columns>}, id = $id, <precondition> } as _eligible
%if _eligible
%then
    { <write rule, which repeats the precondition>
      :put board_items { ... } }
    { ?[..., applied] := *board_items{...}, id = $id, applied = true } as _applied
    %return _applied
%end
{ ?[..., applied] := *board_items{...}, id = $id, applied = false } as _unchanged
%return _unchanged
```

**The precondition appears twice on purpose.** The `%if` decides whether the
write block runs at all; the write rule's own body decides which rows it
touches. Stated once, a mistake in either place alone produces a write that
ignores prior state. Stated twice, it does not.

**`applied` is returned from inside the transaction that decided it**, never
from a preflight read. That is the entire feature: two agents racing for one
item both see an unclaimed row if they are allowed to look first. And the loser
gets the authoritative row back alongside `applied: false`, so it can see *who*
holds the item rather than merely being refused.

### What the race test actually proves

`two_claimants_race_and_exactly_one_wins` (`board/board_tests.rs`) runs
twenty-five rounds, two threads released from a barrier, and asserts that
exactly one caller is told it won and that the stored `claimed_by` is that same
caller. It also tracks how many threads were inside `claim_board_item`
simultaneously and **fails if no round ever reached two** — a concurrency test
that never achieved concurrency passes for the wrong reason.

Be precise about what that establishes. The callers race; the *writes* are then
serialised by the process-wide guard in `archon-cozo`, which `run_mutable` holds
across the whole script — a process mutex, a cross-process file lock, and
SQLITE_BUSY retry. So the test does not show two transactions interleaving.

The CAS is still load-bearing rather than decorative, for two reasons. The
second transaction reads a non-null `claimed_by` and refuses, which is the
mechanism producing exactly one winner — serialisation alone would just make
both writes succeed in order. And the write lock is re-entrant: a thread already
inside a guarded operation reuses the lock it holds rather than blocking on it
(`crates/archon-cozo/src/locking.rs`, "reusing Cozo write lock already held by
this thread"; covered by
`blocking_write_lock_is_reentrant_inside_a_guarded_operation` in
`crates/archon-cozo/src/tests/write_lock.rs`). A nested guarded operation
therefore short-circuits the lock entirely, leaving the in-transaction check as
the only protection.

`release_board_claim` reverts `claimed` to `open` but leaves any status further
along the lifecycle alone: releasing the agent is not the same as retracting the
work it already recorded. `set_board_item_status` is conditional on `from` still
holding, because the reviewer marking an item `resolved` and the parent marking
it `escalated` are separate agents acting on separate reads, and an
unconditional write would let the later one erase a verdict it never saw.

## Board items are never pruned

The garden does not know the relation exists. Nothing decays a board item,
prunes it for staleness, prunes it for overflow, or merges it into another.

This is the point of not making board items memories. A handoff that evaporates
after thirty days of decay is worse than no handoff at all: the agent that
raised it is gone, the work it recorded is not done, and there is now no record
that it was ever outstanding. Structural immunity is better than a rule someone
has to remember not to break.

`garden_leaves_the_task_board_untouched`
(`crates/archon-memory/tests/garden_tests.rs`) runs consolidation with
deliberately destructive settings — staleness zero days, an importance floor
above any real value, `max_memories: 1` — asserts that the memory count actually
fell, so the garden provably ran, and then that the board row is unchanged.

## `BoardAccess`, not `MemoryTrait`

Board operations are a separate trait. There are seventeen `impl MemoryTrait`
across four crates, most of them test doubles and stubs; six new required
methods would have broken all of them for no benefit, since nothing that mocks
memory needs a board.

`BoardAccess` is implemented for all three ways a process reaches the graph
(`access/board_impl.rs`):

| Implementor | Path |
|---|---|
| `MemoryGraph` | direct, in the process that owns the CozoDB writer |
| `MemoryClient` | JSON-RPC over the memory server socket |
| `MemoryAccess` | dispatches to whichever of the two this process has |

The remote arm is not optional. CozoDB admits one writer, so every Archon
process after the first reaches memory over TCP — a direct-only board would
silently be a private board. Claims resolve in the one process that owns the
writer, which is what keeps the compare-and-set global rather than per-process.

## Enumerating runs

Every read described so far starts from a `run_id`, because every writer has
one: a subagent inherits its parent's. A reader arriving from outside the run —
a dashboard, an operator asking what is outstanding — has no such handle, and
until `list_board_runs` there was no way to obtain one. It returns each distinct
`run_id` with its per-status counts, a total, and the newest `updated_at` across
the run, ordered most recently touched first.

It is deliberately a relation scan folded in Rust, where every other read goes
through `board_items:by_run`. That index is keyed by `run_id`, so it can answer
"which items are in this run" but not "which runs exist" — the distinct keys are
exactly what an index lookup needs supplied. Nothing on the hot path calls it:
the drain gate and the agents looking for work all arrive holding a `run_id`
already. Paying a scan on a view that renders once per poll is the cheaper
mistake than a second relation every board write would have to keep in step.

Statuses a run has no items in are **absent** from the counts rather than
present as zero, so a caller reads presence directly.

Like every other board operation it is on all three access paths and on the RPC
dispatch table. A read that only worked in-process would not surface as an
error: a second Archon process would see a board with no runs on it and report
that there was nothing to do — which is the shape of issue #128, where one
memory operation missing from the RPC surface read as an empty result.

## The web read view

`crates/archon-sdk/src/web/board.rs`, three read-only endpoints:

| Route | Answers |
|---|---|
| `GET /api/board/runs` | every run with items, most recently touched first |
| `GET /api/board/runs/{run_id}/items` | that run's items, oldest first; optional `?status=open,claimed` |
| `GET /api/board/items/{item_id}/history` | one item's transitions, oldest first |

An unrecognised name in `?status=` is a 400 rather than a silent empty result:
ignoring it would answer with the whole board while the caller believed it had
filtered. The page is `web/src/views/BoardPage.tsx`, polled with react-query —
the board is a snapshot whose items change status in place, not an append-only
log, so there is nothing to stream.

**These are not gated on attached mode, and `/api/agents/live` is.** That
endpoint reads `BACKGROUND_AGENTS` and `TASK_MANAGER`, which own `JoinHandle`s
and cannot cross a process boundary, so it is meaningful only inside the session
it reports on. The board is not a registry — it is rows in the memory database —
so a standalone `archon web` shows the real board.

Reaching it is the part that needs care. `inspect.rs` reuses
`WebRuntimeHandles::memory`, the handle the host session already has open; the
board cannot, because `BoardAccess` is deliberately off `MemoryTrait` and no
`BoardAccess` can be recovered from an `Arc<dyn MemoryTrait>`. What it must not
do instead is open the database itself: CozoDB admits one writer, and in
attached mode the host holds it on that very file. `WebBoardStore` goes through
`open_memory_with_db_path` — the same singleton election every other entry point
uses, which reads `memory.port` and connects as a client when a server answers,
and only otherwise takes `memory.lock`. `MemoryAccess` implements `BoardAccess`,
which is what makes the elected handle directly usable. The result is `Direct`
when the web server is the only process and `Remote` over TCP when a session
owns the writer.

Issue #134 was a direct `MemoryGraph::open` here, and the reason it survived
review is worth recording: **it does not fail.** Measured on Windows with the
sqlite backend, a raw second open of a database a live session holds returns in
206ms from a genuinely separate OS process and reads correct rows — no hang, no
error, no wrong answer. Nothing at the filesystem or CozoDB layer enforces the
single-writer rule; the election is the only thing that does. So every
behavioural test of these endpoints passes with the bug in place, and the
regression guard has to assert which arm was elected rather than what came back
(`the_store_connects_as_a_client_when_a_server_already_holds_the_writer`).

The election result is cached for the life of the server; a failure is not,
because the memory server can be down when the first request arrives and up a
minute later. The database's absence is checked before the election, because
`open_memory_with_db_path` creates what it cannot find and looking at a
dashboard must not be what brings a memory database into existence — that case
answers `storeAvailable: false`, which the page distinguishes from an empty
board.

## A claim lasts as long as its holder

A claim has no TTL. It is valid while the agent holding it is still executing,
and the sweep in `crates/archon-tools/src/board/leases.rs` releases the rest.

`holder_liveness` is one lookup against `BACKGROUND_AGENTS`, keyed by the
runtime subagent id — the same id the board records as the claim holder. It
used to be a fan-out over `TASK_MANAGER` and then `BACKGROUND_AGENTS`, because
`TaskCreate` and `AgentTool` happened to record their agents in different
places. `archon-pipeline` recorded in neither, so its agents read as dead from
birth and had their claims released while they worked, and a fan-out has to
grow an arm for every spawn path added afterwards or fail exactly that way
again, silently.

So the registration moved to the one function every runner passes through,
`run_subagent_with_auto_background` in `crates/archon-tools/src/agent_tool/run.rs`.
Liveness is now a property of having been spawned. Two details make it hold:

- The registration is released when the **runner** ends, not when
  `run_subagent` returns. Those differ on the `AutoBackgrounded` arm, where the
  agent keeps working after the call returns — and where `SubagentStop` never
  fires, which is why nothing hook-based can be trusted with this.
- `TOP_LEVEL_AGENT` is in no registry and is alive as long as the process is,
  so it is answered before the lookup. Reading its absence as death would have
  the sweep strip its own claims.

`TASK_MANAGER` is untouched by this and still owns task status, metadata and
`/tasks`. It is simply no longer asked whether an agent is alive.

## The drain gate

A run is not complete while its board has outstanding items. The gate reads the
board at the run's terminal point and refuses to accept a final report over
anything still open.

Where it sits depends on the lifecycle:

- The **relay path**, which is the default, applies it in `run_final_gates`.
- The **orchestrated path** (`ARCHON_ORCHESTRATED_LIFECYCLE=1`, opt-in) does not
  pass through `run_final_gates`, so the gate lives at its own terminal point:
  the `FinalReport` action reads the board before the terminal checkpoint.

On the orchestrated path it **refuses** rather than failing the run, and that
difference is the point. The orchestrator is still inside its action loop, so it
is told what is outstanding and can resolve, promote or decline it. If it
cannot, the action budget exhausts and the run ends at
`orchestrated-budget-exhausted` with no accepted report — the honest outcome.

**A board that cannot be read refuses; a run with no board configured passes.**
"Unreachable" and "empty" are the same silence from the gate's position, and
reading that silence as success is the whole reason the gate exists. A gate that
reports a clean run because it could not see the board produces the same record
an enforced run produces, which makes it worse than no gate at all.

`process_board_drain` therefore always names a board rather than returning
`Option`. A process without one gets `UnreachableBoardDrain`, whose read fails
with the reason, and `LifecycleDriver` distinguishes the three cases: reads
clean, cannot be read, none configured. The last exemption is right for
`archon-workflow`, which has consumers with no memory at all, and wrong for this
binary, where every production entry point installs a board and its absence
means something broke.

## Where the board is installed

`BoardHandle::Global` is resolved once per process, and every entry point that
can run agents installs it. There are three sites, and which one runs is
determined by how the process was invoked:

| Entry point | Installed by |
|---|---|
| Interactive TUI | the TUI bootstrap |
| `--print`, `--headless`, and every non-interactive session | `src/session/build_agent_board.rs`, from `config.memory.db_path` |
| `archon workflow` (a subcommand, which builds no session) | `src/command/workflow_live_board.rs`, at the top of `run_live_cli_action` |
| Standalone `archon web` | `WebBoardStore`, which elects its own handle |

All of them go through the same singleton election, and both halves of that
matter. A guessed default data dir would give the run a private board that
accepts writes nobody reads. A raw `MemoryGraph::open` would bypass the one
thing that enforces CozoDB's single writer — and, as the #134 note above
records, would not fail while doing so.

The workflow site is deliberately in `run_live_cli_action` and not
`run_live_action`, which the TUI also reaches: there the board is already
installed from a `MemoryAccess` this process holds, and a second
`open_memory_with_db_path` against a database it already owns is exactly the
bypass the election prevents.

The three sites cannot race. `main` returns straight out of
`handle_subcommand_if_present`, so a process running a workflow subcommand
reaches neither `build_agent.rs` nor the interactive bootstrap; `main_modes`
exits from the print and headless arms before the interactive path; and
`run_live_cli_action` runs once per invocation. `workflow_live_board.rs` still
resolves `BoardHandle::Global` before opening anything, because the *test*
binary can reach all three from one process, and a `OnceLock` that silently kept
the first handle would hide a second unelected open rather than prevent it.

### What has no board

A bare subcommand other than `workflow`, a test binary that installed none, and
a session or workflow whose memory would not open — logged once and left
uninstalled, because a run does useful work without a board.

For the tools that is a truthful "the task board is unavailable" rather than a
refusal to start. For a decomposed-PRD run it is not free: the run reaches the
drain gate, the gate reports why it could not check, and the run ends
`needs_review` rather than accepted. A run whose completion could not be checked
has not been shown to be complete.

## See also

- [Learning systems](learning-systems.md) — the garden, consolidation, and why memories decay
- [Spawn everything](spawn-everything-philosophy.md) — how subagents are used
- [Data locations](../operations/data-locations.md) — where the memory database lives
