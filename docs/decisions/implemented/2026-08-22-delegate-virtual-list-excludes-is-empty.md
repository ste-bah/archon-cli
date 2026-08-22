# Implemented: `is_empty` is deliberately excluded from `delegate_virtual_list!`

- **Status:** Implemented
- **Date:** 2026-08-22
- **Area:** TUI list overlays — `crates/archon-tui/src/virtual_list.rs`
- **Decided in:** [`c48af3d79`](https://github.com/ste-bah/archon-cli/commit/c48af3d79) — `refactor(tui): give the list overlays one cursor delegation and one render tail`

## The decision

`delegate_virtual_list!(field, ItemType)` generates the seven cursor methods that
ten overlay screens had each written out by hand as one-line forwards to the
`VirtualList` they wrap: `len`, `selected_index`, `selected`, `move_up`,
`move_down`, `page_up`, `page_down`.

`is_empty` is the obvious eighth. It is not generated, and it must not be added.

## Why

Three of the ten screens answer `is_empty` from a **separate backing `Vec`**, not
from the list. Their `is_empty` and their `len` are questions about two different
collections, and that is deliberate in each case — the list holds the currently
filtered or currently loaded rows, the backing vector holds everything.

A macro that generated `is_empty` would compile in all ten screens and change the
answer in three of them. Nothing would fail. The screens would simply start
reporting emptiness about a collection they had never been asking about, in the
one situation — a filter that matches nothing over a non-empty corpus — where the
two disagree and where the difference is the whole point.

That is the general rule this record exists to state:

> **A refactor is only a refactor if every generated body is the body it replaced.**
> When a name means slightly different things at different call sites, generating
> it unifies the meaning as a side effect. That is a behaviour change wearing a
> refactor's clothes, and it is invisible in review precisely because the diff is
> a deletion.

The seven that *are* generated pass this test: all ten screens implemented them
character for character against the same field, so the generated body is provably
the body it replaced. The evidence is that all ten screens kept their render-buffer
assertions and model tests unchanged across the extraction.

## Consequences

- Each screen keeps its own hand-written `is_empty`, and the reader is expected to
  look at which collection it consults. The doc comment on the macro says so, so
  the next person to notice the "missing" eighth method finds the reason at the
  point of confusion rather than in this file.
- The seven-method macro is a slightly odd-looking API. That is the correct cost.

## How to apply this elsewhere

Before adding a member to any delegation macro, `impl` generator, or shared trait
default, check every existing call site for one that answers the question from a
different source. If one does, the member does not belong in the shared shape —
or the call site's divergence is itself a bug that must be fixed and *proved*
first, on its own, in a change that can fail.

## See also

- [Rejected: raise the TUI duplication threshold above 5%](../rejected/2026-08-22-raise-the-tui-duplication-threshold.md)
  — the decision that produced this extraction.
- [`docs/defensive-patterns.md`](../../defensive-patterns.md)
