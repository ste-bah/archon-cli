# Proactive Session Briefing

Proactive briefing gives the agent useful context before it repeats an avoidable mistake. It combines memory, reasoning-quality warnings, pending behavior proposals, and world-model readiness into the first-turn briefing.

## Preview It

```bash
archon briefing preview --task "fix the failing macOS trainer test"
```

Inside the TUI:

```text
/briefing preview --task "fix the failing macOS trainer test"
```

The preview is safe. It reads local stores and prints what would be injected.

## What Can Appear

| Section | Source | Example |
|---|---|---|
| Memory | Memory garden | Prior project facts and stable preferences. |
| Reasoning warnings | `~/.archon/reasoning-quality` | "Codebase claim before source read in this area." |
| Pending proposals | Governed learning | Behavior proposal waiting for approval. |
| World model | `~/.archon/world-model` | Cold-start status or advisory readiness. |

The briefing is capped by `learning.session_briefing.max_chars` and ranks reasoning warnings by severity, recency, and relevance to the task text.

## Configuration

```toml
[learning.session_briefing]
enabled = true
include_memory = true
include_reasoning_quality = true
include_pending_behaviour_proposals = true
include_world_model = true
max_items = 8
max_chars = 4000
world_model_requires_ready = true
```

Disable a section rather than disabling the whole briefing if the signal is noisy.

## Policy

```toml
[policy.reasoning_quality]
allow_session_start_injection = true
allow_behavior_proposal_generation = true
```

If session-start injection is denied, Archon still records reasoning-quality events; it simply does not inject them into prompt context.

## Operational Pattern

1. Run normal sessions with reasoning quality enabled.
2. Inspect captured signals:

```bash
archon reasoning status
archon reasoning patterns
archon reasoning claims <session-id>
```

3. Preview future briefing output:

```bash
archon briefing preview --task "review provider runtime changes"
```

4. If shadow mode is holding trust updates, label samples:

```bash
archon reasoning shadow-report
archon reasoning sample-label <session-id>
```

## Failure Posture

Briefing is best effort. Missing stores, cold world-model state, or policy denial do not block the session. Archon starts normally and records the reason in logs or status output.
