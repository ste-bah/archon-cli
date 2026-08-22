# Rejected: wire `forget_session` at `finish_session`

- **Status:** Rejected
- **Date:** 2026-08-22
- **Area:** read-before-write freshness — [`crates/archon-tools/src/file_observation.rs`](../../../crates/archon-tools/src/file_observation.rs), [`src/session_loop/session_shutdown.rs`](../../../src/session_loop/session_shutdown.rs)
- **Chosen instead:** per-agent release at subagent completion
- **Implementation status:** on `fix/observation-lifecycle-and-vacuous-tests`; not merged at the time of writing

## What was proposed

`ObservationRegistry` grows without bound (see the
[sibling record](2026-08-22-bounded-lru-for-the-observation-registry.md)). There is
already a `forget_session(session_id)` that drops an entire session's records
including its subagents'. There is already a function called `finish_session` that
runs when a session loop ends. The proposal writes itself: call the first from the
second, one line, done.

## Why it was turned down

**`finish_session` runs where it cannot help, and does not run where it is needed.**

Its two callers are both in `src/session_loop/mod.rs`, and both are process-exit
positions:

1. `mod.rs:146` — the unix-only early bail when SIGTERM handler registration fails.
   The next statement is `return`.
2. `mod.rs:334` — the last statement of the session loop's async block, whose value
   propagates straight back to `main`.

In both, the process is on its way out within milliseconds. Freeing a `HashMap`
immediately before the OS reclaims the whole address space is not a fix; it is a
gesture. Every byte the proposal recovers was about to be recovered anyway.

Meanwhile the paths that genuinely accumulate sessions in one long-lived process
never reach it:

- **The workflow live runner** mints a distinct session id *per stage per retry
  attempt* — `workflow_agent_session_id` returns
  `{run_id}-stage-{stage}-attempt-{n}` — so one workflow run leaves N_stages ×
  N_attempts `Observer` entries behind. It does not call `finish_session`, and
  `archon-workflow` does not even depend on `archon-tools` (the manifest documents
  avoiding that edge to break a dependency cycle), so nothing on that path *can*
  release observations.
- **The ACP server** and `ide-stdio` mint one session id for the entire process
  lifetime and serve unbounded `session/prompt` turns against it, each spawning
  subagents with their own `Observer`s. Neither calls `finish_session`, and neither
  fires `SessionEnd`.

So the fix would have worked only in the two places where it did not matter, while
looking — in a diff, in a review, in a commit message — like it had solved the
whole thing. **That is the specific harm being avoided: not that the change is
useless, but that a useless change in the shape of a complete one stops anyone
looking for the real fix.** The registry would still have grown without limit on
every long-lived path, and the issue would have been closed.

There is also a plain factual error underneath the proposal, which is worth
recording because it is the kind that survives review. `finish_session` does not
touch the registry and never could: it fires `HookType::Stop` and `StopFailure`.
The registry is released from the `SessionEnd` handler in
`crates/archon-core/src/hooks/registry.rs`, and `SessionEnd` is fired from exactly
two production sites, both interactive slash commands — `/exit` and `/clear`. The
similar names describe unrelated seams. Naming a function after the concept its
neighbour handles is enough to make a wrong wiring look obviously right.

## What was done instead

Release per agent at `AgentSubagentExecutor::handle_inner_complete`, the
subagent-completion path that always runs — success, failure and cancellation.
That is where entries are actually created in bulk, and it is a boundary that
occurs thousands of times inside a live process rather than once as it dies.

## Follow-on defect found while deciding this

`/clear` fires `SessionEnd` with the **same** session id and then continues
in-process. The session's observations are therefore wiped while the agent is still
running, so its next `Edit` to a file it had already read returns
`Verdict::Unobserved` and, under the default `read_before_edit = "block"`, a
refusal. This is a live instance of the same failure the LRU was rejected for, on a
path unrelated to subagents. It is out of scope for that change and is recorded
here so it is not rediscovered from scratch.

## What would change this

If a future entry point were added that loops over many sessions inside one process
*and* routed shutdown through `finish_session`, calling `forget_session` there would
be correct and cheap. The objection is not to the call; it is to the claim that the
call closes the issue. Any such change must state which multi-session path it
covers and which it does not.

## See also

- [Rejected: bound `ObservationRegistry` with an LRU](2026-08-22-bounded-lru-for-the-observation-registry.md)
- [`docs/defensive-patterns.md`](../../defensive-patterns.md) — DP-7.
