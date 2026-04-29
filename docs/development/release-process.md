# Release process

archon-cli ships from `main` with `--no-ff` merge commits per PR. Versions are bumped in the workspace `Cargo.toml`, tagged, and noted in `docs/release-notes/`.

## Versioning

Semver. Pre-1.0 means breaking changes can land in minor versions; archon-cli is currently 0.1.x.

## Per-PR steps

### 1. Implement and verify

Pass all 6 [dev flow gates](dev-flow-gates.md):

```bash
scripts/dev-flow-gate.sh TASK-ID
```

### 2. Bump the workspace version

Workspace root `Cargo.toml`:
```toml
[workspace.package]
version = "0.1.29"
```

All workspace crates inherit via `version.workspace = true`.

### 3. Update docs

- `docs/release-notes/v0.1.29.md` — release notes for this version
- `docs/README.md` — link to the new release notes
- Any user-facing doc that drifts from the change (slash commands, tools, config keys)

### 4. Commit and push to feature branch

```bash
git checkout -b feat/my-change
git add ...
git commit -m "feat(...): brief subject

Body explaining the why and how, referencing relevant TASK-IDs and PRs.
"
git push -u origin feat/my-change
```

### 5. Open PR

```bash
gh pr create --title "feat(...): brief subject" --body "$(cat <<'EOF'
## Summary
- Bullet 1
- Bullet 2

## Test plan
- [x] cargo test --workspace -j1: ALL PASS
- [x] cargo fmt --check: PASS
- [x] cargo build --release -j1: PASS
- [x] Live smoke test: <description>
EOF
)"
```

### 6. Audit before merge

Per [dev flow](dev-flow-gates.md), the audit pattern:
- Verify file scope (what's in the diff matches the spec)
- Run integration tests independently
- Confirm release build with fresh binary mtime + version SHA
- Run `cargo fmt --all -- --check`
- Manual smoke if user-facing

### 7. `--no-ff` merge

```bash
git checkout main
git merge --no-ff <branch> -m "Merge PR (#NN): v0.1.29 brief subject"
git push origin main
```

Sequential `--no-ff` merges (NOT fast-forward) — preserves revert points if a regression surfaces.

### 8. Tag the release

```bash
git tag v0.1.29
git push origin v0.1.29
```

### 9. Build the release binary

```bash
cargo build --release --bin archon -j1
./target/release/archon --version
# Expected: archon 0.1.29 (<short-sha>)
```

If you hit `petgraph::graphmap::NeighborsDirected::next` ICE:
```bash
cargo clean -p petgraph -p archon-pipeline
cargo build --release --bin archon -j1
```

### 10. Skip CI for docs-only PRs

Add `[skip ci]` to the commit subject for docs-only changes:

```bash
git commit -m "docs: split README into structured docs/ tree [skip ci]"
```

GitHub Actions natively skips workflow runs when the commit message contains any of: `[skip ci]`, `[ci skip]`, `[no ci]`, `[skip actions]`, `[actions skip]`.

For PRs with paths-ignore filters in the workflow (e.g., `tui-observability.yml`), workflows already auto-skip if no matching paths changed.

## Workspace test suite

Before any release, run the full workspace tests:

```bash
# WSL2
cargo nextest run --workspace -j1 -- --test-threads=2

# Native Linux/macOS
cargo nextest run --workspace
```

Pre-existing failures (4 known: checkpoint, login, resume) are documented and don't block. New failures must be fixed or explicitly ignored with rationale.

## Hotfix releases

For urgent bug fixes:
- Branch from `main`
- Pass all 6 gates (no skipping for "small" fixes)
- Bump patch version (0.1.28 → 0.1.28.1 if needed, or 0.1.29 if it's the next normal version)
- `--no-ff` merge

## Rollback

If a regression slips past the audit:

```bash
# Revert the merge commit
git revert -m 1 <merge-sha>
git push origin main

# Bump version (revert is a forward change)
# Update Cargo.toml to 0.1.30 or similar
```

The `--no-ff` merge structure makes revert clean — `git revert -m 1` undoes the entire feature without disturbing other PRs.

## Pre-release tags

For experimental builds:
```
v0.1.30-rc1
v0.1.30-beta.2
```

These don't auto-update homebrew/cargo install paths.

## See also

- [Contributing](contributing.md)
- [Dev flow gates](dev-flow-gates.md)
- [Release notes](../release-notes/) — the actual release log
