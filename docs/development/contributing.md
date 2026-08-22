# Contributing

## Workflow

1. Fork the repo on GitHub
2. Clone your fork, branch from `main`
3. Make changes; add tests
4. Run the CI gate locally (`scripts/ci-gate.sh`)
5. Open a PR

## Dev environment

- Rust 1.85+ (edition 2024)
- `cargo`, `rustfmt`, `clippy` (bundled with Rust)
- Optional: `cargo-nextest` for faster test runs
- Optional: `lld` linker for faster builds
- WSL2: see [Installation](../getting-started/installation.md#wsl2-caveat--parallelism-limit)

## Git hooks (one-time setup after clone)

```bash
./scripts/install-git-hooks.sh
```

Sets `core.hooksPath = scripts/git-hooks`. Active hooks:
- **pre-push**: runs `cargo fmt --all -- --check` (strict — blocks on drift) then `cargo clippy --workspace --all-targets -j1` (advisory — warnings emit but only compile errors block, matching CI semantics). Bypass with `git push --no-verify` (rare; only when CI on a feature branch is the right place to catch the issue).

Hook scripts are tracked in the repo, so updates land via `git pull` — no manual copy to `.git/hooks/`.

## Code style

- `cargo fmt --all` before every commit
- No `unwrap()` / `expect()` outside tests; use `anyhow::Result` or typed errors
- Files under 500 lines, functions under 50 (enforced by Gate 2 auto-check)
- No `#[allow(...)]` to suppress warnings — fix the underlying issue
- Comments explain WHY, not WHAT (well-named code self-documents the WHAT)

## Testing

- TDD: write the failing test before the implementation
- Tests near the code: `#[cfg(test)] mod tests` inside the file, or `tests/` for integration
- Mock external deps (network, file system, time)
- Integration tests for cross-crate behavior in `crates/<crate>/tests/`
- **Assert the outcome, never the elapsed time.** Before writing a test that touches a subprocess, a platform difference, or a clock, read [`docs/defensive-patterns.md`](../defensive-patterns.md) — every rule there is traced to a [postmortem](../postmortem/README.md) of a check in this repo that reported green while inspecting nothing.

## CI gates

archon-cli's CI flow is `scripts/ci-gate.sh` — 8 technical gates (file-size, banned-imports, R0 entry gate, fmt, clippy, test, baseline diff, bench compile-check). Run locally before pushing:

```bash
./scripts/ci-gate.sh                # full
./scripts/ci-gate.sh --skip-bench   # faster iteration
```

See [CI gates](dev-flow-gates.md) for the full step list and rationale.

Judge every gate by its **exit code**. Counting matches in a command's output tests the output, not the command: a tool that is missing, unauthenticated, or silently reformatted still yields a number, and zero is indistinguishable from clean. That is [postmortem 0004](../postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md).

## Cargo discipline

WSL2 only:
```bash
cargo build --release -j1
cargo nextest run --workspace -j1 -- --test-threads=2
```

Native Linux/macOS: omit `-j1`.

## PR review

PRs are reviewed for:
1. Tests cover the change (Gate 4)
2. Sherlock-style adversarial review surfaces no concerns (Gate 6)
3. Documentation updated for any user-facing change (slash commands, tools, config keys)
4. No drift introduced (e.g., README count claims still match code)

## Doc updates

If you change anything user-facing, update the relevant `docs/` page in the same PR. Drift is a Gate 6 fail.

## See also

- [Dev flow gates](dev-flow-gates.md)
- [Adding a tool](adding-a-tool.md)
- [Adding a skill](adding-a-skill.md)
- [Adding an agent](adding-an-agent.md)
- [Release process](release-process.md)
- [Defensive patterns](../defensive-patterns.md) — rules for writing checks that cannot lie
- [Postmortems](../postmortem/README.md) — the incidents those rules came from
- [Decision records](../decisions/README.md) — including the `rejected` bucket
