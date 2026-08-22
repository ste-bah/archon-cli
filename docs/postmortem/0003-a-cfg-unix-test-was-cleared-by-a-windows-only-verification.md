# 0003 — a `#[cfg(unix)]` test was cleared by a Windows-only verification

- **Date discovered:** 2026-08-22, by CI on PR [#211](https://github.com/ste-bah/archon-cli/pull/211)
- **Introduced:** [`bed66e1a0`](https://github.com/ste-bah/archon-cli/commit/bed66e1a0) — 2026-08-22 07:07
- **Fixed:** [`3d2c24b58`](https://github.com/ste-bah/archon-cli/commit/3d2c24b58) — 2026-08-22 09:13
- **Exposure:** 2 hours 6 minutes, and one push of a broken assertion to a PR
- **Defect class:** [**verification that could not observe its subject**](../defensive-patterns.md#dp-10--a-verification-must-run-where-the-thing-it-verifies-exists)
- **Test:** `tool_round_timeout_kills_bash_process_group`, `crates/archon-core/src/subagent/runner/tests/parallel.rs`

## What happened

`bed66e1a0` was itself a fix for [postmortem 0002](0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md):
it replaced wall-clock assertions with outcome assertions across three suites. In
the subagent runner it wrote:

```rust
assert!(
    message.contains("(cap: 5s)"),
    "the run must end on the subagent's own 5s cap, not the tool's 60s one: {message}"
);
```

The fixture configures a **2 second** cap. `SubagentRunner::new` takes `max_turns:
u32` then `timeout_secs: u64`, and the call passes them adjacent:

```rust
"mock".into(),
5,                              // max_turns
2,                              // timeout_secs — the wall-clock cap
Arc::new(AgentConfig::default()),
```

The `5` is the number of turns. The message being matched is built at
`crates/archon-core/src/subagent/runner/runtime.rs`:

```rust
"Subagent wall-clock timeout: {elapsed}s elapsed (cap: {}s) during tool round at turn {}/{}",
self.timeout_secs,
turn,
self.max_turns,
```

so the real runtime string contains **both** numbers, and the assertion reached
past the one it wanted to the one beside it. The actual CI panic shows them in a
single sentence:

```
the run must end on the subagent's own 5s cap, not the tool's 60s one:
Subagent wall-clock timeout: 2s elapsed (cap: 2s) during tool round at turn 0/5
```

There is a plausible origin for the wrong number. The assertion this replaced was
`assert!(started.elapsed() < Duration::from_secs(6));`. Rewriting a `< 6s` bound
into a named cap, `5` is the number that feels right and `2` is the number the
fixture had always said.

## Why the verification cleared it

The PR body listed its evidence, all by exit code:

> `cargo check --workspace --all-targets` 0 · `cargo test -p archon-core -p archon-tui` 0 · `cargo test --bin archon` 0 (1866 passed) · `cargo fmt --all -- --check` 0 · FileSizeGuard 0 over 500 · arch-lint 0 · jscpd 0

Every one of those was true as run. None of them could see this assertion.

**On Windows the test does not exist.** The item is `#[cfg(unix)]`:

```rust
#[cfg(unix)]
#[tokio::test]
async fn tool_round_timeout_kills_bash_process_group() {
```

`cfg` is applied during expansion, *before* name resolution and type-checking, so
on a `windows-msvc` target the whole item — string literal included — is removed
from the AST. It is not compiled, not type-checked, not run. The Windows test-count
spread proves it: 13361 tests on ubuntu, 13355 on macos, **13236** on windows.
`cargo test -p archon-core` exiting 0 on Windows is a true statement about 13236
other tests.

*(Correction to how this was first described: the **file** is not `cfg(unix)` — it
compiles on Windows and contains ungated tests that do run there. Four individual
items are gated. The distinction matters: "the file does not compile on Windows"
is false, and "the test does not exist on Windows" is true.)*

**On WSL, `cargo check` cannot evaluate a string.** The one Linux command in the
list was `cargo check --workspace --all-targets`, which does expand `#[cfg(test)]`
modules and does type-check the item. But `"(cap: 5s)"` is a `&'static str` and
`String::contains(&str)` type-checks for any two strings. A literal's *content* is
opaque to the compiler: there is no const-evaluation of `contains`, and no lint
compares an assertion's expected substring against the `format!` that produces the
runtime value. The only thing that can distinguish `"(cap: 5s)"` from
`"(cap: 2s)"` is executing the test and comparing bytes.

So the verification matrix had a hole exactly the shape of the defect: the platform
that ran the tests did not have the test, and the platform that had the test only
type-checked it.

CI found it in the first run — three red jobs, ubuntu and macos on this assertion,
windows on the [0002](0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md)
hook test. Every non-`build + test` check, arch-lint included, passed.

## The fix

Name the number once, where the compiler can enforce the tie:

```rust
/// The subagent's own wall-clock cap for this run, named so the constructor
/// argument and the assertion below cannot drift apart. It must stay well under
/// the 60s `BashTool` timeout, because the whole point is proving which of the
/// two deadlines ended the run.
#[cfg(unix)]
const SUBAGENT_WALL_CLOCK_CAP_SECS: u64 = 2;
```

used both as the constructor argument and to build the expected substring:

```rust
let expected_cap = format!("(cap: {SUBAGENT_WALL_CLOCK_CAP_SECS}s)");
assert!(
    message.contains(&expected_cap),
    "the run must end on the subagent's own cap, not the tool's 60s one: {message}"
);
```

A magic number that appears twice can disagree with itself. A constant cannot.
This does not fix the verification hole — it removes the class of defect the hole
was hiding.

## What to do about the hole itself

The honest constraint is that a single-platform developer machine cannot verify
platform-gated tests, and pretending otherwise is how this happened. Two things
follow, and both are cheap:

1. **Before claiming a test suite passes, know which tests your platform skipped.**
   `cargo test` reports a count. If a change touches a `cfg`-gated item, the local
   run is not evidence about it, and the claim must say so.
2. **CI is the verification for gated code.** Not a formality after the claim —
   the thing the claim is waiting on.

## Rules this produced

- [DP-10 — a verification must run where the thing it verifies exists](../defensive-patterns.md#dp-10--a-verification-must-run-where-the-thing-it-verifies-exists)
- [DP-11 — `cargo check` is not evidence about a value](../defensive-patterns.md#dp-11--cargo-check-is-not-evidence-about-a-value)
- [DP-12 — a number that must match another number is a named constant](../defensive-patterns.md#dp-12--a-number-that-must-match-another-number-is-a-named-constant)
