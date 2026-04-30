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
- WSL2: see [Installation](../getting-started/installation.md#wsl2-caveat)

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

## CI gates

archon-cli's CI flow is `scripts/ci-gate.sh` — 7 technical gates (file-size, banned-imports, fmt, clippy, test, baseline diff, bench compile-check). Run locally before pushing:

```bash
./scripts/ci-gate.sh                # full
./scripts/ci-gate.sh --skip-bench   # faster iteration
```

See [CI gates](dev-flow-gates.md) for the full step list and rationale.

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
