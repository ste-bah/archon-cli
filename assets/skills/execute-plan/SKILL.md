---
name: execute-plan
description: Use when a written plan or task list exists and the work needs building — several tasks, more than one sitting, or a job too large to hold in one context. Runs each task in a fresh subagent, reviews it, and keeps a ledger that survives compaction.
---

# Executing a plan

Long builds fail for a boring reason: context. By task nine the model is
carrying eight tasks' worth of dead detail, and quality degrades in a way that
is invisible from inside.

The fix is to stop accumulating. Each task gets a fresh agent that knows only
what that task needs. What carries between them is written down, not
remembered.

## Prerequisites

A plan with discrete tasks. If you do not have one, stop and make one —
`/compose-pipeline` builds the chain, or `/spec-to-tasks` refines an existing
tree. Executing a vague plan produces vague work faster.

Isolated work belongs in a worktree: spawn with `isolation: "worktree"` so
parallel tasks cannot overwrite each other, and declare `intended_writes` so
overlaps surface at spawn instead of at merge.

## The loop

For each task, in order:

### 1. Brief

Extract *that task's* text. Not the whole plan. The implementer should not be
reasoning about task 7 while doing task 3 — that is the context bloat you are
avoiding, reintroduced through the brief.

Include: the task, interfaces established by earlier tasks, and any constraint
that binds all of them.

### 2. Implement

Spawn a subagent with the brief. It builds, and it verifies its own work
(`/verify-done`). It does not review itself and does not spawn its own
reviewer — you do that, because an agent grading its own homework grades
generously.

### 3. Review

Spawn a *separate* agent against the diff. Two questions, both required:

- **Does it do what the task said?** Spec compliance.
- **Is it any good?** Correctness, edge cases, whether it fits the codebase.

Give the reviewer the brief and the diff. Not the implementer's report as
fact — its account of what it did is a claim, and the diff is the evidence.

### 4. Fix, bounded

Findings go back to the implementer. It still has context, so this is cheap.

Cap it at five rounds. Rounds one to three, resume the same agent. Rounds four
and five, spawn a fresh one with a stronger model — if three attempts failed,
the context is now part of the problem.

**At five rounds, stop and decide.** Fix it yourself, cut the scope, or park it
with a reason on the board. What you must not do is loop silently or quietly
drop the finding. Record the ruling either way.

### 5. Record

Append the outcome to the board before moving on. This is the ledger, and it
is the whole recovery story: if the session compacts, or you crash, or you come
back tomorrow, the board plus git history says where you were. Memory does not.

## Why the board, not a file

The board is a real store with status transitions, so a task's state is
queryable rather than parsed out of prose, every agent in the run sees the same
thing, and `gaps_remain` items block the turn from ending until they are closed.

A markdown ledger has none of that, and it goes stale the first time someone
forgets to update it.

## When to stop and ask

Four cases, and only four:

- Something irreversible (force-push, dropped data, published artefact)
- A security-relevant change
- An effect outside the worktree
- A plan so wrong that continuing means guessing

Everything else: decide, record the decision, keep going. Stopping to ask about
a decision you could have made and written down is how a two-hour autonomous
run becomes a two-hour conversation.

## Next

All tasks done and reviewed? `/verify-done` over the whole branch, then
`/land-branch`.
