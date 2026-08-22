# Defensive patterns

Rules for writing checks, tests and verification steps in this repository. Each is
traced to the [postmortem](postmortem/README.md) that produced it — the number in
brackets is the incident, and the incident is where the reasoning and the evidence
live. If a rule seems excessive, read its postmortem before relaxing it; every one
of them describes something that already happened here, usually for months.

Read this before writing a gate, a lint, a CI step, or a test that involves a
subprocess, a platform difference, or a clock.

---

## The rule the others are instances of

### DP-0 — a check whose scan target can vanish must fail, not pass

*(from [0001](postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md),
[0002](postmortem/0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md),
[0003](postmortem/0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md),
[0004](postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md),
[0005](postmortem/0005-a-documented-page-was-never-committed-because-git-add-skips-ignored-paths.md)
— all of them)*

Every automated check has a subject: a region of code, a running process, a set of
rows, a platform. Almost every such check is written to detect *violations*, so its
clean state is "found nothing". And in almost every case, "the subject is missing"
produces exactly the same nothing.

That collapse is the single defect class behind every incident recorded here:

| Incident | The subject | How it vanished | What the check said |
|---|---|---|---|
| [0001](postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md) | a marked region of `src/main.rs` | a refactor carried the markers to another file | `all checks passed`, for 128 days |
| [0002](postmortem/0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md) | a hook subprocess | no `sleep` and no `/tmp` on Windows, so it never spawned | passed in ~0 ms, for 136 days |
| [0003](postmortem/0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md) | a `#[cfg(unix)]` test | `cfg`-stripped before compilation on the verifying platform | `cargo test` exit 0 |
| [0004](postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md) | a jscpd duplication report | jscpd wrote none, or one covering zero files | `duplication = 0.00%` … `PASS`, for 128 days |
| [0005](postmortem/0005-a-documented-page-was-never-committed-because-git-add-skips-ignored-paths.md) | a file being staged | matched an ignore rule, so `git add` skipped it | exit 0, and a commit message naming it |

**So: a check must be able to distinguish "I looked and found nothing wrong" from
"I did not look."** Concretely — declare the subject, assert it is non-empty, and
fail if it is not. Silence is not a pass.

The three follow-on questions to ask of any check you write:

1. What is the subject, and what happens to this check if it moves or disappears?
2. Would this check still pass if the thing it tests never ran at all?
3. Can someone reading the output tell how much was inspected?

If the answers are "it passes", "yes", and "no", you have written a check that will
eventually lie, and the lie will be indistinguishable from success.

---

## Gates, lints and scanners

### DP-1 — a lint must verify its scan target resolves before reporting on it

*(from [0001](postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md))*

The specific form for scanners. A lint that greps a file, a directory, a marked
region, or a set of function names must verify that target resolves to something
before reporting on it. An empty scan is a broken lint, not a clean tree, and it
exits non-zero saying so:

```bash
vacuous() {
    echo "arch-lint: RULE ${1} HAS NOTHING TO SCAN — ${2}" >&2
    exit 1
}
```

Never `warn but don't fail`. `scripts/lint/arch-lint.sh` carried the comment
`# No markers = can't scope; warn but don't fail (markers might be in transit)` for
four months, during which the transit had long since arrived somewhere else.

### DP-2 — every scanner reports what it inspected, and its test asserts the count is non-zero

*(from [0001](postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md))*

Exit 0 from a scanner is a statement about violations found, not about work done.
The two are indistinguishable from outside unless the scanner says how much it
looked at. Print a denominator:

```
arch-lint: rule=1 files=24 sites=226 name=no .await on agent work in input handler (D1)
```

and have the guarding test parse it and assert both numbers are greater than zero.
A test that only asserts `output.status.success()` is satisfied by a script that
reads no files.

### DP-3 — anchor a scan to a structural target, not to a comment

*(from [0001](postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md))*

A `BEGIN`/`END` comment pair travels with the code it annotates, to somewhere the
lint is not looking — and nothing breaks when it does. Prefer a target whose
absence is loud: a directory, a module path, a symbol the region must contain. Add
an anchor assertion that proves the region is still the code the rule is about
(arch-lint requires `spawn_turn` to appear in the input-handler directories).

If a comment marker is genuinely the only way to express the region, the rule must
still **fail** when the marker is absent, and its test must still assert a non-zero
inspected count. See the [decision record](decisions/rejected/2026-08-22-restore-the-input-handler-markers.md)
for why restoring the markers was rejected.

### DP-4 — an `#[ignore]`d test is not a plan

*(from [0001](postmortem/0001-arch-lint-inspected-nothing-and-reported-green.md))*

`input_handler_markers_exist` was `#[ignore]`d pending issue #224, and there is no
issue #224 in this repository — the numbering has not reached it. The one test that
would have caught four months of drift was disabled, waiting on a ticket that does
not exist, and its presence in the file made the gap look managed.

Either the test runs, or it is deleted and the reason is written where the next
reader will look. If it is ignored, the `#[ignore]` reason must name a real,
open issue — and that is a temporary state with an owner, not a filing system.

---

## Tests

### DP-5 — assert the outcome, never the elapsed time

*(from [0002](postmortem/0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md))*

`assert!(elapsed < N)` is satisfied by every way of failing fast. The faster and
more completely the subject breaks, the more confidently the assertion passes.

Assert what the system *did*: the error names its phase, the result reports the
kill, the hook was started and then cut short.

```rust
// No.
assert!(elapsed.as_secs() < 4, "expected the hook to be killed within ~2s");

// Yes.
assert_eq!(result.skipped_count, 0, "the hook must have been started, not skipped");
assert!(result.block_reason().unwrap_or_default().contains("timed out"));
```

An elapsed-time bound that merely restates an outer `tokio::time::timeout` proves
nothing the timeout would not already have proved; delete it. If timing genuinely
is the property under test, prefer extracting the arithmetic into a process-free
function and unit-testing that (`crates/archon-core/src/hooks/registry/budget.rs`
is the model) over timing the machine.

### DP-6 — before adding eviction, ask what the code does when the entry is gone

*(from [decision: bounded LRU for the observation registry](decisions/rejected/2026-08-22-bounded-lru-for-the-observation-registry.md))*

If "missing" and "never existed" are the same value to the reader, eviction is not
a memory optimisation — it is a silent behaviour change with a size-dependent
trigger, and which call it breaks depends on what else happened to be in the map.
Bound a cache; release a record on a lifecycle boundary.

### DP-7 — a fix that only works where the problem does not occur is worse than no fix

*(from [decision: wire `forget_session` at `finish_session`](decisions/rejected/2026-08-22-wire-forget-session-at-finish-session.md))*

The harm is not that the change is useless. It is that a useless change in the
shape of a complete one closes the issue and stops anyone looking for the real fix.
When wiring a cleanup to a lifecycle hook, enumerate the callers and say plainly
which paths the fix covers and which it does not.

Beware similar names describing unrelated seams: `finish_session` fires `Stop`, not
`SessionEnd`, and never touches the registry that `SessionEnd` releases.

### DP-8 — a test must fail if its subject never started

*(from [0002](postmortem/0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md))*

Spawning a process, opening a connection, or launching a task can fail before the
behaviour under test begins. Assert positively that it began. `let _result = ...`
discards the value that carries that evidence — bind it and check it.

### DP-9 — no `/tmp`, no `sleep`, no bare POSIX path in a cross-platform test

*(from [0002](postmortem/0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md))*

`PathBuf::from("/tmp")` on Windows is a rooted path with no drive: it resolves to
`\tmp` on the current drive, which does not exist, and a spawn with a nonexistent
cwd fails with `os error 267` before the command is looked up. Use
`std::env::temp_dir()`.

A test that needs `sleep`, `kill`, process groups, or any other POSIX facility is
`#[cfg(unix)]`, and the gate is stated with a reason. `tests/no_hardcoded_tmp_path_gate.rs`
exists to enforce the path half of this.

### DP-12 — a number that must match another number is a named constant

*(from [0003](postmortem/0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md))*

`(cap: 5s)` was asserted against a fixture configured with `2`, because `5` was the
`max_turns` argument sitting on the line above `timeout_secs`. A magic number that
appears twice can disagree with itself; a constant used in both places cannot.

```rust
const SUBAGENT_WALL_CLOCK_CAP_SECS: u64 = 2;
// ...used both as the constructor argument and to build the expected substring.
let expected_cap = format!("(cap: {SUBAGENT_WALL_CLOCK_CAP_SECS}s)");
```

This applies with double force to substrings matched against a `format!` elsewhere
in the tree, because nothing in the compiler relates the two.

---

## Verification and claims

### DP-10 — a verification must run where the thing it verifies exists

*(from [0003](postmortem/0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md))*

`#[cfg]` is applied before type-checking. On a platform the gate excludes, the item
is not compiled, not checked, and not run — `cargo test` exit 0 is a true statement
about the other 13,236 tests. The Windows/Linux/macOS test counts in this workspace
differ by over a hundred.

Before claiming a suite passes, know which tests your platform skipped. If a change
touches a `cfg`-gated item, your local run is not evidence about it; say so, and
wait for CI. This is not a formality after the claim — it is the thing the claim
depends on.

### DP-11 — `cargo check` is not evidence about a value

*(from [0003](postmortem/0003-a-cfg-unix-test-was-cleared-by-a-windows-only-verification.md))*

`cargo check` stops before codegen and can only tell you an expression is
well-typed. `message.contains("(cap: 5s)")` type-checks exactly as well as
`message.contains("(cap: 2s)")`. String contents, arithmetic results, and matched
substrings are runtime properties. Only `cargo test` / `cargo nextest run`
distinguishes them.

Also: `cargo check` without `--all-targets` does not expand `#[cfg(test)]` modules
at all.

### DP-13 — judge by exit code, never by counting matches in output

*(from [0004](postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md))*

Grepping a command's output tests the output, not the command. If the command did
not run, was not on `PATH`, errored, wrote to stderr, changed its format, or
printed nothing, the grep still returns a number — and `0` is indistinguishable
from a real clean result.

The live example in this tree is in the coding-pipeline agent prompts, under a
heading marked `MANDATORY`:

```bash
# No: if `npm run typecheck` crashed, the file is empty or absent, grep finds
# nothing, and the agent reports zero type errors and declares the pass complete.
cd $TARGET_DIR && npm run typecheck 2>&1 | tee /tmp/phase6-typecheck.txt
TYPE_ERRORS=$(grep -c "error TS" /tmp/phase6-typecheck.txt || echo "0")
```

The tool's own status is the verdict. `grep -c` exits 1 on no match — a real
signal, discarded by `$(...)`. When you genuinely must parse, as
`scripts/check-tui-duplication.sh` must because jscpd does not encode the
threshold in its exit code, then: establish the tool answered, require the output
to be parseable, **assert a non-zero denominator**, and print it (DP-2). That gate
now reports `duplication = 1.13% over 147 files / 34719 lines` rather than a bare
percentage, because 0% over 0 files used to read the same as a clean tree.

### DP-14 — *(retired)*

This number held "a verification step that is not committed did not happen". The
rule was sound but its only evidence was an uncommitted script outside this
repository, so it failed the standard the series is built on: a numbered
postmortem must be an incident in this codebase, verifiable from this history.
0004 was rewritten around a committed gate instead, and the rule was retired with
it rather than left resting on evidence nobody can read.

The number is left as a gap on purpose. Renumbering would break every reference,
and the gap records that a rule was removed rather than inviting a new one to be
written here without a source.

### DP-15 — never turn a tool failure into a benign message

*(from [0004](postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md))*

An absence of findings and an absence of problems are different facts, and this
shape reports the first as the second:

```bash
# The gate printed this, then printed PASS and exited 0, for 128 days.
else
  printf 'TuiDuplicationGuard: no report file generated\n'
fi
printf 'TuiDuplicationGuard: PASS\n'
exit 0

# Same file: a missing python3 or a changed schema became a printed word.
DUP_PCT=$(python3 -c "..." 2>/dev/null || echo "unknown")
```

`|| true`, `|| echo`, and `2>/dev/null` on a command whose exit status is the
signal all have this property. So does a `PASS` printed outside the `if` that was
supposed to guard it.

A second illustration, worth knowing because the numbers are surprising:
`gh pr checks || echo 'No PR checks available'` prints that sentence when checks
are **failing** (exit 1), when they are **pending** (exit 8), when the PR does not
exist (exit 1), and when `gh` is not installed (exit 127) — every state except
all-green, which is the only one where it is suppressed.

Suppress a failure only when you have named the specific one you expect and
established that every other failure still propagates.

---

## Cross-cutting

### DP-16 — a threshold is a claim, not a dial

*(from [decision: raise the TUI duplication threshold](decisions/rejected/2026-08-22-raise-the-tui-duplication-threshold.md))*

Moving a gate's threshold in response to a specific red converts it permanently
into a number that means "whatever makes today's build pass" — and the next change
argues from that precedent. Before touching one, apply the test: shown the specific
findings, would a competent reviewer say the code should change? If yes, change the
code. A threshold change is defensible only when the *measurement* is wrong, and
that case is argued on the measurement, in a [decision record](decisions/README.md).

### DP-17 — a generated body must be the body it replaced

*(from [decision: `is_empty` excluded from `delegate_virtual_list!`](decisions/implemented/2026-08-22-delegate-virtual-list-excludes-is-empty.md))*

When a name means slightly different things at different call sites, generating it
unifies the meaning as a side effect — a behaviour change wearing a refactor's
clothes, invisible in review because the diff is a deletion. Before adding a member
to a delegation macro, trait default, or `impl` generator, check every call site
for one that answers the question from a different source.

---

### DP-18 — a link in a committed document must point at a committed file

*(from [0005](postmortem/0005-a-documented-page-was-never-committed-because-git-add-skips-ignored-paths.md))*

`git add` on a path matching an ignore rule is a **silent no-op with exit status
0**. The file stays on your disk, looking committed, and is absent from every
clone. `docs/providers/bedrock.md` was linked from three pages for nine days and
had never existed in the repository, because `/docs/*` ignored its directory and
nothing said so.

Checking that a link target exists on disk is not enough — that check passes on the
machine where the file is untracked. `tests/docs_cross_references.rs` asserts the
target is **tracked by git**. When adding documentation under a new directory,
verify with `git status --ignored` or `git check-ignore -v <path>` before trusting
that `git add` did anything.

### DP-19 — writing the hazard down is not fixing it

*(from [0005](postmortem/0005-a-documented-page-was-never-committed-because-git-add-skips-ignored-paths.md))*

`.gitignore` in this repository carried an accurate diagnosis of this exact failure
— "`git add` skips ignored paths silently and nothing warns that a tracked page
points at an untracked file" — written when it cost fifteen broken image links. The
comment was correct, the one-directory fix was applied, the mechanism was left
alone, and it fired again on a different directory nine months later.

A comment records what happened. A check prevents it. When you find yourself
writing an explanation of a trap next to a workaround for one instance of it, ask
what would fail if someone stepped in it again — and if the answer is "nothing",
that is the actual fix.

## See also

- [Postmortems](postmortem/README.md) — the incidents these rules came from.
- [Decision records](decisions/README.md) — including the `rejected` bucket, which
  is where "we already considered that" is written down.
- [`docs/architecture/spawn-everything-philosophy.md`](architecture/spawn-everything-philosophy.md)
  — the architectural rules `arch-lint` enforces.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — the gates every change must pass.
