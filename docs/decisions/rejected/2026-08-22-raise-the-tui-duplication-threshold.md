# Rejected: raise the TUI duplication threshold above 5%

- **Status:** Rejected
- **Date:** 2026-08-22
- **Area:** TUI duplication gates — `scripts/ci/check-duplicate-code.sh`, `scripts/check-tui-duplication.sh`
- **Decided in:** [`c48af3d79`](https://github.com/ste-bah/archon-cli/commit/c48af3d79) — `refactor(tui): give the list overlays one cursor delegation and one render tail`
- **Chosen instead:** extract the two duplicated shapes

## What was proposed

Duplication over `crates/archon-tui/src` is capped at 5% (NFR-TUI-MOD-003,
AC-MOD-04). Adding the permission-preset selector screen took it from 4.97% to
**5.07%** and the gate went red.

> **Which gate.** There are two, over the same directory, and they measure
> differently — an earlier draft of this record named the wrong one.
> `scripts/ci/check-duplicate-code.sh` runs jscpd at its default clone length and
> is the one that produced the numbers in this record;
> `scripts/check-tui-duplication.sh` passes `--min-lines 20` and reads much lower.
> Re-measured on the tree today: **4.18%** at the default length, **1.13%** at
> `--min-lines 20`. The 4.18% sits right beside the 4.02% the commit reports, which
> is what pins the attribution.
>
> The enforcement is inverted, which matters for how much this decision was worth:
> the sensitive gate that went red is `continue-on-error: true` in
> `tui-observability.yml` and cannot fail a build, while the gate wired into
> `ci.yml` without that flag is the one reading 1.13% — comfortably clear of 5% and
> so not much of a constraint. See
> [postmortem 0004](../../postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md),
> which is about the second gate reporting `PASS` over an empty scan.
>
> None of this changes the decision below. The duplication was real at either
> setting, and it was extracted rather than legislated away.

The proposal was to raise `THRESHOLD` — to 6, or to 5.5, or to whatever cleared the
current number. The supporting argument was reasonable on its face: the new screen
was not badly written, it was written the same way the nine screens beside it were
written, and a gate that fails on consistency is punishing the wrong thing.

## Why it was turned down

The gate was measuring something true.

The tenth screen tipped the number, but it was not the defect. The defect was that
there was one right way to forward a keypress to a cursor and ten places to write
it out by hand, and one right way to end a list render and ten places to get it
wrong. jscpd counted three clone pairs against the new file — two against
`theme_screen`, one against `settings_screen` — and six more pairs among the nine
that already existed. The number had been climbing toward the line for nine
screens; the tenth is just where it arrived.

That duplication had already cost something concrete. Three of these screens
shipped without a visible selection ([#192](https://github.com/ste-bah/archon-cli/issues/192)),
because the highlight symbol is part of the six-line render tail that each screen
copied independently and three copies dropped it. Raising the threshold would have
bought silence on the measurement that predicted that bug, immediately after it
happened.

The general shape of the objection: **a threshold is a claim about how much of a
thing is acceptable, not a dial for making today's build pass.** Moving it in
response to a specific red converts it into the second, permanently — the next
screen argues from this precedent, and the number after that is 6.

## What was done instead

The two shapes moved to where they belong:

- `delegate_virtual_list!` in
  [`crates/archon-tui/src/virtual_list.rs`](../../../crates/archon-tui/src/virtual_list.rs)
  generates the seven cursor methods (`len`, `selected_index`, `selected`,
  `move_up`, `move_down`, `page_up`, `page_down`) from the field name and the row
  type.
- `overlay::render_list` and `overlay::render_table` in
  [`crates/archon-tui/src/overlay.rs`](../../../crates/archon-tui/src/overlay.rs)
  take the items or rows and the selected index and build the widget, attach the
  block and the shared highlight, point a state at the selected row, and render —
  so a screen can no longer be written that forgets the highlight symbol.

Duplication over `crates/archon-tui/src` fell from 5.07% to **4.02%** — below the
4.97% the tree carried *before* the preset selector existed. The threshold is
untouched and nothing is excluded.

The behaviour evidence is that every one of the ten screens kept its render-buffer
assertions and its model tests unchanged.

## What would change this

The argument above is specifically that this red was true. It is not a claim that
5% is a law of nature. A threshold change is arguable when the *measurement* is
wrong rather than the code — for example if jscpd's `MIN_LINES=20` window started
counting a family of derive-like blocks that cannot be deduplicated in Rust
without a macro that hurts readability more than the duplication does. The test to
apply is whether a competent reviewer, shown the specific clone pairs, would say
"those should be one thing." Here they would; that is why the number moved instead
of the line.

## See also

- [Decision: `is_empty` is deliberately excluded from `delegate_virtual_list!`](../implemented/2026-08-22-delegate-virtual-list-excludes-is-empty.md)
  — the boundary of the extraction this decision produced.
- [Postmortem 0001](../../postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md)
  — the neighbouring failure, where a gate that had stopped measuring anything
  reported green instead of red.
- [`docs/maintainability/refactor-map.md`](../../maintainability/refactor-map.md)
