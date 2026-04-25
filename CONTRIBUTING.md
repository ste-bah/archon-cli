# Contributing to archon-cli

Thanks for contributing. This document is a short pointer to the rules and
checks that guard the codebase. Read them once before your first PR.

## Development flow

- Every task in `project-tasks/**` MUST pass all six dev-flow gates via
  `scripts/dev-flow-pass-gate.sh`. No exceptions.
- Tests are written before implementation (Gate 1). Sherlock-holmes review is
  required at Gate 3 and Gate 6. No self-attestation at Gate 5 — use `--exec`.
- `cargo` invocations on this repo MUST run with `--jobs 1` and
  `-- --test-threads=2`. Parallel rustc / test processes crash WSL2.

## Architectural Guidelines

Before writing or reviewing async code, read:

- [`docs/architecture/spawn-everything-philosophy.md`](docs/architecture/spawn-everything-philosophy.md)
  — the "spawn-everything, never block the event loop" philosophy (D10) and
  its three rules: no `.await` >100ms in the main event handler, producer
  channels are unbounded, tools own task lifecycle.

The rules are enforced mechanically in CI by `scripts/lint/arch-lint.sh`
(workflow job `arch-lint`). The lint is a backstop, not a substitute for
understanding the rules during review.

## Reporting bugs / proposing changes

- File issues with a reproduction on `main` and include OS + Rust toolchain
  version.
- Large refactors go through `project-tasks/**` with a PRD, functional spec,
  and technical spec before any code lands.
