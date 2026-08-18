---
name: verify-done
description: Use before telling anyone the work is finished — after a fix, a feature, or a refactor, and any time you are about to write "done", "working", or "should be fine". Turns a claim into evidence by running the project's real checks and recording anything still broken.
---

# Verify before claiming done

The most common failure is not a bad fix. It is a fix that was never run.

"Should work" and "does work" are different claims, and only one of them is worth
making. This skill is the difference.

## The rule

**A completion claim needs a command and its output.** Not a reading of the
diff, not a description of what the change does. Something ran, and you saw
what it printed.

If you cannot produce that, say what you actually did: "changed X, not yet
run". That is a useful, honest report. "Done" is not.

## Process

### 1. Say what "working" means

Before running anything, write down what you expect. One line.

> The parser accepts the empty-array case and the 14 existing parser tests
> still pass.

Doing this first is what stops you from reading whatever the output says as
success. A prediction you wrote before you looked is falsifiable; a
rationalisation you formed after is not.

### 2. Run the project's checks — the real ones

Not a check you invented because it is fast. The one the project uses:
its test command, its linter, its build, its gate script. `/ci-gate-walker`
runs archon's if you are working in this repo.

Scope it to what you touched if the full suite is slow, but never substitute a
weaker check and report it as the strong one.

### 3. Compare against step 1

Three outcomes, and only one of them is finished:

- **Matches your prediction.** Done. Say what you ran.
- **Fails.** Not done. Fix it, then start again at step 2.
- **Passes, but not for the reason you predicted.** Treat this as a failure.
  A green run you cannot explain is a green run that is not testing your
  change — the classic case being a test that never executed the new path.

### 4. Record what you could not close

Anything still broken, unverified, or knowingly deferred goes on the board with
`BoardRaise` — status `gaps_remain`, with the evidence and what "done" would
look like.

This is not bookkeeping. Archon's completion gate reads exactly this: while an
item sits in `gaps_remain` for this run, the turn will not end, and the
findings come back to you to work off. Fix and close each with `BoardResolve`,
or decline it with a reason if it turns out not to be worth acting on. An
unexplained open gap keeps the turn from finishing, which is the point.

Set `skills.completion_gate = "warn"` if you want it to complain rather than
block, or `"off"` to disable it.

## What does not count as verification

- Reading the diff again
- The code compiling
- A test you wrote but did not watch fail first — see `/tdd`
- A subagent reporting success you did not check
- "The change is small"

The last one is the most dangerous, because it is usually true and still
irrelevant. Small changes break builds constantly.

## Next

Verified and on a branch? `/land-branch` to merge or open a PR.
