# `scripts/tests/`

Tests for the **gate scripts in `scripts/`**. Nothing else belongs here.

## The rule

A script in this directory must be one of:

1. **A self-test of a gate `scripts/` ships** — proof that the gate can go red.
   A gate nobody has watched fail is indistinguishable from a gate that
   cannot fail, and a silently-permissive gate is worse than no gate.
2. **A behavioural test of a shell script** the project ships to users, where
   the behaviour has no other test (`archon-init-test.sh` runs
   `scripts/archon-init.sh` against a temp directory and inspects the tree).
3. **A check that CI actually invokes a gate.** The compiler cannot see a
   YAML job. This is the one thing grep is genuinely good for here.

Anything about **Rust source** belongs in a `#[test]`, not here.

## Why the rule exists

This directory used to hold 31 scripts. Twenty-four of them were per-task
"structural verifiers" that grepped hardcoded paths for `pub fn <name>` to
prove some slice had been wired up. They were deleted in one commit, because:

- **They proved nothing the compiler doesn't.** `render/mod.rs` calls
  `draw_skills_menu`; delete that function and the build fails. A gate that
  greps for its name adds no coverage — it only restates the call site in a
  weaker language.
- **They created the dead code they cited.** Two wrappers in
  `src/command/plan_file.rs` existed solely so a gate could match `pub fn
  plan_audit_path`. Nothing called them, so the module needed
  `#![allow(dead_code)]` to compile quietly. The gate manufactured its own
  evidence.
- **They were wrong far more often than they were right.** When they were
  deleted, 8 of the 31 were failing. Every one of the 8 was a false alarm:
  a file split under the 500-line `check-file-sizes.sh` guard, or a symbol
  renamed. Zero real defects, across every failure. The repo's real gate was
  breaking its fake ones.
- **Almost nothing ran them.** Only two were referenced by `ci.yml`. The other
  29 had no call site anywhere in the tree, so their results — true or false —
  were never seen by anyone.

The one thing they checked that a compiler cannot is whether a wired slice has
regressed to a stub. That check survives as
[`scripts/check-deferral-markers.sh`](../check-deferral-markers.sh): one
tree-wide scan with an allowlist, rather than two dozen per-task greps at
hardcoded paths.

## What is here

| Script | Kind | Run by |
|---|---|---|
| `test_check_file_sizes.sh` | self-test of `check-file-sizes.sh` | `ci.yml`, `ci-gate.sh` |
| `test_ci_baseline_diff.sh` | self-test of the baseline test-list diff | `ci.yml`, `ci-gate.sh` |
| `preserve-invariants-self-test.sh` | self-test of `check-preserve-invariants.sh` | `ci.yml` |
| `archon-init-test.sh` | behavioural test of `scripts/archon-init.sh` | `ci.yml`, `ci-gate.sh` |
| `ci-preserve-invariants-wired.sh` | asserts `ci.yml` invokes the gate | `ci.yml`, `ci-gate.sh` |
| `r0-entry-gate-wired.sh` | asserts `ci.yml` + `ci-gate.sh` invoke the R0 gate | `ci.yml` |
| `archon-core-machete-clean.sh` | unused-dependency check for `archon-core` | manual (needs `cargo-machete`) |

Every one of these is invoked by something. **If you add a script here and
nothing runs it, you have added a file, not a check.**
