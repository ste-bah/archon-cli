# 0004 — a CI watcher reported ALL GREEN because its tool was not on PATH

- **Date discovered:** 2026-08-22
- **Exposure:** one PR pushed and reported clean while three required jobs were red
- **Defect class:** [**vacuous check**](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass) — counting matches in output that was never produced
- **Related:** [0003](0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md), which is what the watcher failed to report

## Provenance — read this first

The watcher was an ad-hoc script written during an agent session and never
committed. It is not in the working tree, not in `git log --all`, and the string
`ALL GREEN` has never appeared in this repository's history. **The counting logic
below is reconstructed from the session that ran it, not quoted from an artefact,
and is labelled as such.** Everything in "What is independently verifiable" is
from GitHub and from committed files.

That the evidence is gone is itself part of the lesson: a verification step that
lives only in a shell history cannot be reviewed, cannot be fixed, and cannot be
shown to have been wrong. See DP-14.

## What happened

A polling script watched the checks on a PR and printed a verdict. Reconstructed,
it counted rows in `gh` output:

```bash
# RECONSTRUCTED — not a quotation from a committed file
pending=$(gh pr checks "$PR" | grep -c pending)
failing=$(gh pr checks "$PR" | grep -c fail)
if [ "$pending" -eq 0 ] && [ "$failing" -eq 0 ]; then
    echo "ALL GREEN"
fi
```

On this machine `gh` is installed **only inside WSL**. Invoked from Git Bash it is
not on `PATH`, so each call printed `gh: command not found` to stderr and produced
no stdout. `grep -c` over nothing returns `0`. Both counts were zero, both
conditions held, and the script declared success.

The two failure modes it was written to detect — checks still running, checks
failing — are both encoded as *positive counts*. Their absence means "clean". The
third state, **"I could not look"**, produces exactly the same zeroes as "I looked
and everything is fine", and the script had no way to tell them apart. It was, in
effect, asking "how many failures did I see?" of a program that had never run.

`grep -c` also exits 1 when it matches nothing, which would have been a signal —
but the exit status was discarded by the command substitution, and the value that
survived was the count.

## What is independently verifiable

- **PR [#211](https://github.com/ste-bah/archon-cli/pull/211)** was pushed at head
  `9cebf7177`. CI run `32557998469` failed three required jobs: `build + test` on
  ubuntu, macos and windows. Ubuntu and macos failed on
  `tool_round_timeout_kills_bash_process_group` ([0003](0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md));
  windows failed on `test_per_hook_timeout_clamped_to_remaining_budget`
  ([0002](0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md)).
  Every other check, `arch-lint` included, passed.
- The PR body claimed, in full: *"All by exit code: `cargo check --workspace
  --all-targets` 0 · `cargo test -p archon-core -p archon-tui` 0 · `cargo test
  --bin archon` 0 (1866 passed) · … arch-lint 0 · jscpd 0."* Every one of those
  commands really did exit 0; none of them could observe the defect. That claim
  and those three red jobs coexisted for two hours.
- **The same shape is committed and still live in this repository.**
  `.archon/agents/github/pr-manager.md` instructs an agent to run:

  ```
  gh pr checks || echo 'No PR checks available'
  ```

  which converts every possible failure — `gh` missing, unauthenticated, rate
  limited, network down, PR not found — into a benign sentence and a zero exit.
  An agent following that line reports "no checks available" for a PR with five
  red ones.

## Why nothing caught it

Because the script's output was reassuring and its input was absent, and nothing
in the pipeline compared the two. The stderr line saying `gh: command not found`
was printed on every poll and scrolled past under a verdict that said the opposite.

The general form is worth stating precisely, because it is the most common way an
automated check lies:

> **Grepping a command's output tests the output, not the command.** If the
> command did not run, produced an error, printed to stderr, changed its format,
> or emitted nothing, the grep still returns a number, and that number is
> indistinguishable from a real clean result.

Exit codes do not have this property. A tool that is not on `PATH` returns 127. A
`gh` call against a nonexistent PR returns non-zero. Those are unmistakable —
right up until a `|| echo` or a `$(...)` throws them away.

## The fix

The replacement establishes that it can see, before it says anything about what it
sees:

1. **Resolve the tool or abort.** `command -v gh` must succeed; on this machine
   that means invoking through WSL (`wsl.exe -e bash -lc "gh ..."`) rather than
   assuming the Windows `PATH`.
2. **Require parseable rows.** The check output must yield at least one recognised
   check row. Zero rows is an error — "no checks" and "cannot read checks" are the
   same observation and must both stop the script.
3. **Judge by exit code, never by counting matches.** `gh pr checks` already exits
   non-zero while checks are pending or failing. That status is the verdict; the
   text is for humans.
4. **Report the denominator.** Print how many checks were examined alongside the
   verdict, so a run that inspected nothing is visible in its own output — the
   same discipline [postmortem 0001](0001-arch-lint-inspected-nothing-and-reported-green.md)
   applied to `arch-lint`'s `files=X sites=Y`.

## Rules this produced

- [DP-13 — judge by exit code, never by counting matches in output](../defensive-patterns.md#dp-13--judge-by-exit-code-never-by-counting-matches-in-output)
- [DP-14 — a verification step that is not committed did not happen](../defensive-patterns.md#dp-14--a-verification-step-that-is-not-committed-did-not-happen)
- [DP-15 — never turn a tool failure into a benign message](../defensive-patterns.md#dp-15--never-turn-a-tool-failure-into-a-benign-message)
