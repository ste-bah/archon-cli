# TUI Drain-Stall Warning

The `TuiEvent drain stalled` warning means Archon has queued TUI events that
have not been drained by the render loop within the active threshold.

It does not fire for an idle channel. If no TUI events are pending, a quiet
session is treated as idle, not stalled.

## Thresholds

- Default interactive threshold: 10 seconds.
- Long-running slash workload threshold: 90 seconds.

Slash commands that run mirrored CLI work, such as `/docs ingest <path>`, mark
the TUI as long-running for the lifetime of the spawned task. The marker is
cleared by a drop guard so panics and early returns do not leave the widened
threshold stuck on.

## Duplicate Suppression

After one stall warning, duplicate warnings are suppressed until one of these
happens:

- the pending queue grows;
- the render loop drains another event;
- 60 seconds pass while the stall is still active;
- pending reaches zero, which clears the episode.

## Investigation

When the warning fires, check the structured fields:

- `pending_events`: queued TUI events waiting to be drained;
- `stalled_ms`: time since the last drain;
- `threshold_ms`: active threshold;
- `last_variant`: last drained event variant;
- `total_drained`: process-lifetime drain count.

If `pending_events` keeps growing and `total_drained` does not move, suspect a
blocked render loop. If `pending_events` returns to zero, the warning was a
transient backlog and the TUI recovered.
