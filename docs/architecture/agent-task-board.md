# Agent task board

The task board is how agents in one run hand work and findings to each other. A
reviewer that finds a problem raises an item with the file references and what
"done" would mean; an implementer claims it, works it, and moves it along; the
parent can re-scope it or escalate it. The alternative — passing findings as
prose in a subagent's return value — loses everything the parent does not
happen to quote forward, and gives no way for a second agent to take ownership
of a finding without two agents doing the same work.

This document covers the storage layer, which is what exists today. The rest of
the design is [#125](https://github.com/ste-bah/archon-cli/issues/125); see
[What is not built](#what-is-not-built).

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

`run_id` partitions the board, and it is the partition the drain gate will be
defined over. Every subagent inherits its parent's. `list_board_items_by_run`
goes through `board_items:by_run` rather than filtering a scan, for the polling
reason above.

`kind` is `issue` or `note`, kept apart because the lifecycles differ: an issue
outlives the run that raised it and must resolve, be promoted, or be declined; a
note is context for whoever next touches the area. Conflated, the board fills
with "looked at X, seemed fine" and the drain gate becomes noise.

`status` moves through `open`, `claimed`, `in_review`, `gaps_remain`,
`resolved`, `declined`, `promoted`, `escalated`.

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

## What is not built

Stated plainly, because the storage layer reads as more complete than the
feature is:

- **`round` has no mutator.** It is stored, decoded, and returned, and nothing
  ever advances it. The review loop bounds its own rounds internally, so the
  column is currently a place for an escalation ladder to record itself rather
  than something any code reads.
- **A subagent spawned with an explicit `allowed_tools` list cannot reach the
  board.** `BoardRaise`, `BoardClaim`, `BoardList` and `BoardResolve` are in
  `DEFAULT_TOOLS`, which applies only when neither the spawn request nor the
  agent definition names any tools. Most pipeline agents name their tools, so
  they are excluded. There is no always-allow counterpart to the denylist;
  adding one is a policy change rather than a wiring fix.
- **The drain gate runs only in the V2 lifecycle.** The v3 orchestrated path
  (`ARCHON_ORCHESTRATED_LIFECYCLE=1`) has its own terminal report and does not
  pass through `run_final_gates`, so neither the gate nor the third verdict
  applies there.

`--print` and `--headless` used to head that list. `install_board_access` had
one caller, the TUI bootstrap, and `src/session/build_agent.rs` — which builds
every non-interactive agent — never opened memory, so `BoardHandle::Global`
resolved to nothing and every board call in those modes answered *"the task
board is unavailable: no memory service is open in this process"* (#137). The
tools are in `DEFAULT_TOOLS`, so the model was offered them, tried them, and
always failed.

They now install the handle themselves, in `src/session/build_agent_board.rs`,
from `config.memory.db_path` through the same singleton election. Both halves of
that matter: a guessed default data dir would give the run a private board that
accepts writes nobody reads, and a raw `MemoryGraph::open` would bypass the one
thing that enforces CozoDB's single writer (see the #134 note above). So the
board is reachable from every session surface — the TUI, `--print`,
`--headless`, and a standalone `archon web`, which elects its own handle in
`WebBoardStore`. The two install sites cannot race: `main_modes` exits the
process from the print and headless arms before the interactive path is
reached, so at most one runs per process.

What still has no board is a process with no session — a bare subcommand builds
no agent and installs nothing — and a session whose memory will not open, which
is logged once at startup and leaves the tools reporting the board as
unavailable rather than failing the run.

## See also

- [Learning systems](learning-systems.md) — the garden, consolidation, and why memories decay
- [Spawn everything](spawn-everything-philosophy.md) — how subagents are used
- [Data locations](../operations/data-locations.md) — where the memory database lives
