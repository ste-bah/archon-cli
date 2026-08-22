# Rejected: bound `ObservationRegistry` with an LRU

- **Status:** Rejected
- **Date:** 2026-08-22
- **Area:** read-before-write freshness — [`crates/archon-tools/src/file_observation.rs`](../../../crates/archon-tools/src/file_observation.rs)
- **Chosen instead:** release on a lifecycle boundary (`forget_agent` at subagent completion)
- **Implementation status:** on `fix/observation-lifecycle-and-vacuous-tests`; not merged at the time of writing

## What was proposed

`ObservationRegistry` records what each agent has read, so a later `Edit` or `Write`
can be checked against it:

```rust
pub struct ObservationRegistry {
    seen: Mutex<HashMap<Observer, HashMap<PathBuf, Observation>>>,
}
pub static FILE_OBSERVATIONS: LazyLock<ObservationRegistry> = ...;
```

It is a process-global singleton and it grows on **both** axes without limit: one
outer entry per `Observer` (`session_id` + optional `subagent_id`), one inner entry
per distinct path that observer has read. `execute-plan` runs a fresh subagent per
task, and each gets its own `Observer`, so a long plan accumulates a map per agent
for the life of the process — including for agents that finished hours ago and
whose ids nothing will ever present again.

The standard fix for an unbounded in-memory map is a bounded LRU: cap the entries,
evict the coldest.

## Why it was turned down

**Eviction is safe for an advisory and unsafe for a policy, and this is a policy.**

`Verdict::Unobserved` is produced by a *missing map entry*, and by nothing else:

```rust
let Some(observation) = self.observation(observer, path) else {
    return Verdict::Unobserved;
};
```

So an evicted entry is not "a cache miss we recompute" — there is nothing to
recompute from, because the whole point of the record is that it is the only
evidence the agent looked. Eviction turns `Fresh` into `Unobserved`.

And `Unobserved` is not advisory. `read_before_edit` defaults to `Block`:

```rust
pub enum ReadBeforeEdit {
    /// Refuse the write and say why. The default: a wrong edit that reports
    /// success is worse than a refused one that explains itself.
    #[default]
    Block,
    Warn,
    Off,
}
```

Under that default, an evicted observation produces a refusal:

> `Edit was refused: you have not read <path> in this session, so the text you are
> replacing may not be what is in the file. Read it first, then edit.`

— to an agent that *did* read it, five minutes ago, and is being told otherwise.

The decisive property is not that this is wrong sometimes. It is that **which
write got refused would depend on how many other files happened to be in the map**,
which is not a behaviour anyone can reason about, reproduce, or write a test for.
A cap of 1,000 means a plan touching 1,001 files starts refusing edits, in an
order determined by unrelated reads. The failure is nondeterministic, it looks
exactly like the agent's own mistake, and the message actively misdirects the
reader by asserting something false about their history.

The general rule: **before adding eviction, ask what the code does when the entry
is gone.** If "missing" and "never existed" are the same value, eviction is not a
memory optimisation — it is a silent behaviour change with a size-dependent
trigger.

## What was done instead

Release on a lifecycle boundary, which can only ever drop records nobody will ask
about again:

```rust
pub fn forget_agent(&self, observer: &Observer) {
    if let Ok(mut seen) = self.seen.lock() {
        seen.remove(observer);
    }
}
```

called from `AgentSubagentExecutor::handle_inner_complete` — the subagent-completion
path that always runs, on success, failure and cancellation alike. Scoped to the
one agent, never `forget_session`: the parent is still running and still holds
readings behind edits it has not made yet.

The call site is deliberately **not** the `SubagentStop` hook.
`crates/archon-tools/src/board/leases.rs` already documents why that hook is the
wrong seam for anything that must always run: it fires from `on_visible_complete`,
which the `AutoBackgrounded` arm skips — so the longest-lived agents, holding the
most observations, would be exactly the ones never released.

## Known remaining exposure

This releases *subagent* observations. It does not bound:

- the top-level `Observer` of a long-lived ACP or `ide-stdio` process, which mints
  one session id for the process lifetime and never fires `SessionEnd`;
- the per-stage, per-attempt session ids the workflow live runner mints
  (`{run_id}-stage-{stage}-attempt-{n}`), none of which are ever released.

Both are recorded here rather than fixed, because the correct fix for each is a
lifecycle boundary in that path, not eviction — and inventing one under time
pressure is how the wrong seam gets chosen.

## What would change this

If a future observation store became genuinely advisory — if `Unobserved` degraded
to a warning in all modes, or the store could re-derive an observation from
something durable — then eviction stops changing behaviour and a bound is fine.
The test to apply is the one above: what does the reader do when the entry is
missing? Bound it only when the answer is "the same thing, a little slower."

## See also

- [Rejected: wire `forget_session` at `finish_session`](2026-08-22-wire-forget-session-at-finish-session.md)
  — the sibling proposal, turned down for a different reason.
- [`docs/reference/config.md`](../../reference/config.md) — the `[filesystem]`
  section and the `read_before_edit` default.
- [`docs/defensive-patterns.md`](../../defensive-patterns.md) — DP-6.
