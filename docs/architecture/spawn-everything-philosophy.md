# D10 — Spawn-Everything Architectural Philosophy

**Status**: Guideline (enforced by `scripts/lint/arch-lint.sh` + CI job `arch-lint`)
**Implements**: REQ-FOR-D10 (SPEC-AGS-ARCH-FIXES/US-ARCH-04)
**Anchors**: REQ-FOR-D1, REQ-FOR-D2, REQ-FOR-D3

---

## The Three Rules (US-ARCH-04/AC-02 — verbatim)

1. **no .await >100ms in main event handler** — the TUI input handler and any
   render loop must never synchronously `.await` a long-running task (agent
   execution, tool calls, subagent spawn, HTTP round-trips). Spawn the work with
   `tokio::spawn` and return to the event loop inside one tick.
2. **producer channels are unbounded** — every `mpsc` used to carry
   `AgentEvent`, tool progress, or subagent output must be created via
   `tokio::sync::mpsc::unbounded_channel`. Back-pressure is the **consumer's**
   responsibility (drop-oldest + WARN log at a soft cap such as 10_000
   in-flight events), never the producer's. Bounded producers create latent
   deadlocks when any downstream consumer stalls.
3. **tools own task lifecycle** — `Tool::execute` implementations that start
   background work are the canonical spawn sites. Each spawn registers its
   `JoinHandle` + `CancellationToken` in the global `BACKGROUND_AGENTS`
   registry and returns an `agent_id` synchronously. Upper layers (agent loop,
   input handler) never call `tokio::spawn` on agent work directly; they poll
   the registry or await a completion notification.

---

## Historical smoking gun: `src/main.rs:3743`

Prior to this refactor, the TUI input handler at `archon-cli/src/main.rs:3743`
contained:

```rust
if let Err(e) = agent.process_message(&prompt).await { ... }
```

That single synchronous `.await` was the root cause of every "TUI freeze"
symptom documented in PRD1 Problem 1 and forensic Dimension D1. For the duration
of the agent turn — which can exceed the auto-background threshold of
`AUTO_BACKGROUND_MS = 120_000` — the entire event loop was parked, meaning:

- Keypresses queued until the agent returned.
- Ctrl+C could not be observed by the handler.
- The render loop could not pull events from the agent event channel, which
  itself was a bounded `mpsc::channel(256)`, back-pressuring every producer.

TASK-AGS-106 replaces this call with a `tokio::spawn` wrapper that stores the
`JoinHandle` + `CancellationToken` in an input-handler slot. TASK-AGS-107 wires
Ctrl+C to fire `cancel()` on that token. TASK-AGS-110 activates the arch-lint
grep that guarantees no future PR can reintroduce the pattern.

---

## Why this is a philosophy, not a lint rule

The three rules are architectural invariants: they describe **where**
responsibility lives (consumer owns back-pressure, tool owns spawn, input
handler never blocks), not just which API calls are forbidden. The CI
arch-lint at `scripts/lint/arch-lint.sh` is the mechanical enforcement layer
for rule 1 in the input-handler code path — it is a backstop, not a substitute
for understanding the rules. Reviewers should verify that new async code paths
respect all three rules during review, because the lint only catches the
exact pattern it greps for.

## Rule sequencing note (TECH-AGS-ARCH-FIXES implementation_notes/sequencing)

The three code-level enforcement tasks land in this order:

1. **TASK-AGS-102** — flip the agent event channel to `unbounded_channel`
   (rule 2), because rule 1's spawn wrapper can only be safe when the producer
   side is non-blocking.
2. **TASK-AGS-104 / TASK-AGS-105** — introduce `BackgroundAgentRegistry` and
   move the spawn site into `AgentTool::execute` (rule 3).
3. **TASK-AGS-106** — wrap `main.rs:3743` in `tokio::spawn` (rule 1 in the
   event handler).
4. **TASK-AGS-110** — activate the arch-lint patterns so CI will reject any
   future regression of the smoking gun.

Only after (1)–(3) land can the lint be turned on; activating it earlier would
instantly red-flag the unresolved `.await` at `main.rs:3743`.
