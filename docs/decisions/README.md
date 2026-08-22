# Decision records

A dated note per decision, kept so a decision is not re-litigated from scratch six
weeks later by someone who cannot see why the obvious answer was not taken.

**The `rejected/` bucket is the point of this directory.** `implemented/` largely
duplicates what a good commit message already says. `rejected/` records what was
considered and turned down, and there is nowhere else for that to live: the option
that was not taken leaves no code, no test, and no diff. Without a note it is
invisible, and the next person proposes it again — reasonably, because from where
they stand it looks new.

## States

| Directory | Meaning |
|---|---|
| `proposed/` | Argued for, not yet decided. Short-lived — it moves or it is rejected. |
| `implemented/` | Decided and in the tree. Records the reasoning a diff cannot carry. |
| `rejected/` | Considered and turned down, with the reasoning and what would reopen it. |
| `archived/` | Was true, no longer is — the code it describes has gone. Kept, not deleted, because "we used to do X and stopped" is itself a finding. |

`proposed/` and `archived/` have no notes yet and therefore no directory. Create
one when you have a note to put in it; an empty directory with a placeholder file
advertises a practice that is not happening.

## Why this is not under `.superpowers/sdd/`

`.superpowers/sdd/` holds one file, a task report from 2026-08-14, referenced from
nowhere. It was considered as the home for these and rejected on three grounds:

1. **Wrong shape.** An SDD task report records *what was delivered and which gates
   passed* — deliverables, verification evidence, key files. A rejected decision has
   no deliverable and no gates, because nothing was built. Adding a `rejected/`
   bucket to a task-report tree would mean writing a task report for a task that
   never happened.
2. **Tool-owned path.** `.superpowers/` is where the superpowers SDD workflow
   writes. Hand-authored notes in a directory a tool generates into get interleaved
   with generated output and moved when the tool's conventions change.
3. **Discoverability, which is the whole mechanism.** A dot-directory is invisible
   to `ls`, absent from `docs/README.md`, and unreachable from `README.md`. The one
   report already in there has been unreferenced since the day it was written —
   which is the outcome this practice exists to prevent. `ARCHON.md` also states
   that user-facing markdown belongs in `docs/`.

`.superpowers/sdd/` is left where it is. Its existing report is
[linked below](#prior-art-in-this-repository) so it stops being an orphan.

## Writing one

File name: `YYYY-MM-DD-short-slug.md`, in the bucket matching its state. Moving
between buckets is a `git mv` — the date stays the date of the decision.

The headings are not a rigid template, but a useful record answers:

- **What was proposed** — concretely enough that a reader recognises their own idea
  in it, with the code or config it would have touched.
- **Why it was turned down / chosen** — the actual reason, not the polite one. If
  the reason is a property of the system, state the property; that is what
  generalises.
- **What was done instead**, with paths.
- **What would change this** — the conditions under which the decision should be
  revisited. Without it a rejection reads as permanent, and permanent rejections
  get ignored the moment circumstances shift.

Front matter as a bullet list: status, date, area with a path, the commit or PR
that decided it, and links to related records or postmortems.

**Then link it from the subsystem doc it concerns**, and from the postmortem if it
came out of an incident. A note nobody is routed to is a diary entry.
`tests/docs_cross_references.rs` enforces that every record here is listed on this
page and that every link resolves.

## Rejected

- [Reject referenced content that mentions the wrapper's closing tag](rejected/2026-08-22-reject-content-that-mentions-the-wrapper-tag.md)
  — 2026-08-22, cross-session references. Escaping `<`/`>` was chosen instead;
  detection would let any session make itself unreferenceable by naming the tag.
- [Bound `ObservationRegistry` with an LRU](rejected/2026-08-22-bounded-lru-for-the-observation-registry.md)
  — 2026-08-22, read-before-write freshness. Eviction is safe for an advisory and
  unsafe for a policy: a dropped observation refuses a legitimate write.
- [Wire `forget_session` at `finish_session`](rejected/2026-08-22-wire-forget-session-at-finish-session.md)
  — 2026-08-22, read-before-write freshness. Would have worked only where it did
  not matter, while looking like the whole fix.
- [Raise the TUI duplication threshold above 5%](rejected/2026-08-22-raise-the-tui-duplication-threshold.md)
  — 2026-08-22, maintainability gates. The gate was measuring something true; the
  duplication was extracted instead.
- [Restore the `BEGIN/END INPUT_HANDLER` markers](rejected/2026-08-22-restore-the-input-handler-markers.md)
  — 2026-08-22, architecture lint. A comment pair travels with the code and takes
  the rule with it; a directory list plus vacuity counts was chosen instead.

## Implemented

- [`is_empty` is deliberately excluded from `delegate_virtual_list!`](implemented/2026-08-22-delegate-virtual-list-excludes-is-empty.md)
  — 2026-08-22, TUI list overlays. Three screens answer it from a separate backing
  `Vec`; generating it would be a behaviour change wearing a refactor's clothes.

## Prior art in this repository

- [`.superpowers/sdd/2026-08-14-plan-mode-trust-lifecycle/task-1-report.md`](../../.superpowers/sdd/2026-08-14-plan-mode-trust-lifecycle/task-1-report.md)
  — the plan-contract task report. Task-report shaped rather than decision shaped
  (see above), but it carries a "Known Limitations" section that is decision
  reasoning, and it was previously reachable from nothing.

## See also

- [Postmortems](../postmortem/README.md) — numbered incident writeups. Several of
  these records came out of one.
- [`docs/defensive-patterns.md`](../defensive-patterns.md) — the rules those
  incidents produced.
