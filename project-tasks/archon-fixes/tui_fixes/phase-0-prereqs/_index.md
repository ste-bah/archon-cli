# Phase 0 — Prerequisites (TUI Refactor)

Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-001.md,
TASK-TUI-002.md, TASK-TUI-003.md, TASK-TUI-004.md, TASK-TUI-005.md,
TASK-TUI-006.md, TASK-TUI-007.md, TASK-TUI-008.md, TASK-TUI-009.md,
TASK-TUI-010.md, TASK-TUI-011.md (PRD-TUI-ARCH-002).

Phase 0 is the infrastructure bootstrap for the Rust CLI TUI refactor tracked
under PRD-TUI-ARCH-002. No production code in `crates/archon-tui` or
`src/main.rs` is touched during phase 0. Instead, phase 0 lays down the
baselines, mock harnesses, fixture corpora, and CI guards that every
downstream phase (1..9) depends on. Each TASK-TUI-NNN below is a discrete
unit of work that must pass the full 6-gate dev-flow before it is considered
complete.

## Task catalogue

| ID | Purpose | Depends On | Status |
|----|---------|------------|--------|
| TASK-TUI-001 | Phase-0 index and dev-flow reference docs | — | completed |
| TASK-TUI-002 | File-size allowlist reconciliation | TASK-TUI-001 | completed |
| TASK-TUI-003 | File-size CI gate script + workflow | TASK-TUI-002 | completed |
| TASK-TUI-004 | Banned-imports CI gate script + workflow | TASK-TUI-001 | completed |
| TASK-TUI-005 | Baseline main.rs metrics snapshot | TASK-TUI-001 | completed |
| TASK-TUI-006 | Baseline archon-tui metrics snapshot | TASK-TUI-001 | completed |
| TASK-TUI-007 | Criterion bench harness scaffold (eventloop) | TASK-TUI-005 | completed |
| TASK-TUI-008 | Mock LLM provider harness | TASK-TUI-001 | completed |
| TASK-TUI-009 | TestBackend ratatui harness | TASK-TUI-006 | completed |
| TASK-TUI-010 | Fixture corpus for eventloop/eventchannel phases | TASK-TUI-001 | completed |
| TASK-TUI-011 | Phase-0 closeout + dependency graph validation | TASK-TUI-001..010 | completed |

Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-011.md,
project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/HANDOFF.md,
project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/baselines/phase0-capture-manifest.json.

Phase 0 closed out by TASK-TUI-011 on 2026-04-13T19:55:00+00:00. See
[`HANDOFF.md`](HANDOFF.md) for the downstream phase-1..9 consumption
contract and [`baselines/phase0-capture-manifest.json`](baselines/phase0-capture-manifest.json)
for the hash-pinned machine-readable capture snapshot.

## Phase-id allocation map

Task IDs are allocated in contiguous 100-wide bands so a TASK-TUI-NNN ID
uniquely identifies which refactor phase it belongs to:

| Phase | ID range   | Theme              |
|-------|------------|--------------------|
| 0     | 001..099   | Prerequisites      |
| 1     | 100..199   | Eventloop          |
| 2     | 200..299   | Eventchannel       |
| 3     | 300..399   | Modularization     |
| 4     | 400..499   | Subagent           |
| 5     | 500..599   | Providers          |
| 6     | 600..699   | Slash commands     |
| 7     | 700..799   | Session            |
| 8     | 800..899   | Observability      |
| 9     | 900..999   | Preserve/cleanup   |

## Downstream phases

Phase 0 feeds the following downstream phases. Each downstream phase is
blocked from starting until phase 0 has been signed off by TASK-TUI-011.

- Phase 1 — eventloop
- Phase 2 — eventchannel
- Phase 3 — modularization
- Phase 4 — subagent
- Phase 5 — providers
- Phase 6 — slash
- Phase 7 — session
- Phase 8 — observability
- Phase 9 — preserve

See `dev-flow-reference.md` in this directory for the exact gate-by-gate
handoff contract between phase 0 and each downstream phase.
