# Rejected: restore the `BEGIN/END INPUT_HANDLER` markers to re-enable arch-lint rule 1

- **Status:** Rejected
- **Date:** 2026-08-22
- **Area:** architecture lint — `scripts/lint/arch-lint.sh`, `tests/tc_arch_02_grep_input_handler.rs`
- **Decided in:** [`bed66e1a0`](https://github.com/ste-bah/archon-cli/commit/bed66e1a0) — `Give arch-lint something to look at, and stop three suites timing the machine`
- **Chosen instead:** a directory list plus per-rule vacuity counts
- **Incident:** [Postmortem 0001](../../postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md)

## What was proposed

Rule 1 of `arch-lint.sh` bans `.process_message().await` on the interactive input
path. It scoped itself to the lines between `BEGIN INPUT_HANDLER` and
`END INPUT_HANDLER` comment markers in `src/main.rs`:

```bash
BEGIN_LINE=$(grep -n 'BEGIN INPUT_HANDLER' src/main.rs ... )
END_LINE=$(grep -n 'END INPUT_HANDLER' src/main.rs ... )
if [[ -n "${BEGIN_LINE}" && -n "${END_LINE}" ]]; then
    ...
else
    echo "arch-lint: WARNING — BEGIN/END INPUT_HANDLER markers not found in src/main.rs" >&2
    # No markers = can't scope; warn but don't fail (markers might be in transit)
fi
```

The markers were not there, so the rule inspected nothing and the script reported
success. The repo even carried a test specifying that they should exist —
`input_handler_markers_exist`, `#[ignore]`d pending "AGS-106/107".

The natural fix is the one the existing test already asks for: put the markers back
around wherever the input handler now lives, un-`#[ignore]` the test, and rule 1
starts working again.

## Why it was turned down

**A marker is a comment pair, and a comment pair travels with the code it
annotates — to somewhere the lint is not looking.** The rule leaves with it,
silently, because the failure mode of a missing marker is an empty scan and an
empty scan finds no violations. That is not a hypothetical risk to weigh against
the fix; it is a description of what had already happened, twice, and restoring
the markers restores the mechanism that failed.

The history is short and damning. The markers were added on 2026-04-12
([`bddae1ddd`](https://github.com/ste-bah/archon-cli/commit/bddae1ddd)) together
with the rules that used them. On **2026-04-16** —
[`5f3828a77`](https://github.com/ste-bah/archon-cli/commit/5f3828a77),
`refactor(cli): extract run_interactive_session to session module` — they moved
with their code into `src/session.rs`. The lint hardcodes `src/main.rs`, so rule 1
went vacuous four days after it was switched on. They moved once more into
`src/session_loop/mod.rs`, then were deleted on 2026-05-24
([`0e06d738c`](https://github.com/ste-bah/archon-cli/commit/0e06d738c)).

Note what did *not* go wrong: the markers were carried correctly by the first two
refactors. Nobody was careless. The mechanism failed anyway, because a lint that
locates its region by grepping one hardcoded path cannot follow a region that
moves — and the `else` branch's "markers might be in transit" turned a four-month
absence into a warning on stderr in a passing job.

There is a second reason. `src/main.rs` is now a 113-line argument dispatcher. The
input handler is two directories — `src/session_loop/` and
`crates/archon-tui/src/event_loop/`. Re-marking a region inside a file the code
left would encode a location that is already wrong, and it would have to be
re-marked again the next time either loop moves.

The general form: **prefer a scan target that cannot be removed by accident to one
that can.** A directory either exists or the build is broken. A comment can go
missing while everything still compiles, every test still passes, and the gate
still prints "all checks passed".

## What was done instead

Two changes, and the second is the one that matters.

**1. The region became a directory list.**

```bash
INPUT_HANDLER_DIRS=(src/session_loop crates/archon-tui/src/event_loop)
```

Plus an anchor: the region must still contain a `spawn_turn` call or definition.
A directory that stops existing, or stops dispatching turns, is caught — whereas a
deleted comment was not.

**2. Every rule declares what it is about to scan and refuses to pass if that
target is empty.** Each prints its counts:

```
arch-lint: rule=1 files=24 sites=226 name=no .await on agent work in input handler (D1)
arch-lint: rule=2 files=1  sites=1   name=Agent event transport must await bounded capacity (D3)
arch-lint: rule=3 files=24 sites=30  name=no .await on agent work in input handler function (D1 broad)
```

and `tests/tc_arch_02_grep_input_handler.rs` asserts, for each of rules 1, 2 and 3,
that the line is present and that `files` and `sites` are both non-zero. Exit 0 is
no longer accepted as evidence on its own.

The `#[ignore]`d `input_handler_markers_exist` test was deleted rather than
un-ignored, and the module doc says why, so the next reader does not re-derive the
rejected option.

## What would change this

A marker is defensible when the region genuinely cannot be expressed structurally —
a hot span inside one long function, say, where "the directory" and "the file" are
both too coarse. Even then the rule must fail when the marker is absent, never warn
and continue, and the guarding test must assert a non-zero inspected count. The
objection here is not to markers as such; it is to a scan target whose disappearance
is indistinguishable from cleanliness.

## See also

- [`docs/defensive-patterns.md`](../../defensive-patterns.md) — DP-1, the rule this
  produced.
- [`docs/architecture/spawn-everything-philosophy.md`](../../architecture/spawn-everything-philosophy.md)
  — the D10 rules arch-lint enforces.
