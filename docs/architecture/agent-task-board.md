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

## What is not built

Stated plainly, because the storage layer reads as more complete than the
feature is:

- **`round` has no mutator.** It is stored, decoded, and returned, and nothing
  ever advances it. Deciding which claim starts a new attempt is an
  escalation-ladder policy question and belongs with that half of #125 rather
  than being guessed at here.
- **There are no board tools exposed to subagents.** Nothing outside
  `archon-memory` calls `BoardAccess` today. An agent cannot raise, claim, or
  resolve an item.
- **There is no drain gate.** The `run_id` partition and the `by_run` index
  exist for it, and the status vocabulary anticipates it, but nothing yet blocks
  a run from finishing while items are still open.

## See also

- [Learning systems](learning-systems.md) — the garden, consolidation, and why memories decay
- [Spawn everything](spawn-everything-philosophy.md) — how subagents are used
- [Data locations](../operations/data-locations.md) — where the memory database lives
