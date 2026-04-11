## regen-public-api.sh

Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-011.md
Based on:  00-prd-analysis.md REQ-FOR-PRESERVE-D8 (d), NFR-ARCH-002

Captures deterministic public-API snapshots for the two preserve-D8
crates (`archon-memory` and `archon-core::agents::memory::*`) into
`tests/fixtures/baseline/`. Drift against the committed snapshot is
detected by the integration tests in:

- `crates/archon-core/tests/public_api_snapshot.rs`
- `crates/archon-memory/tests/public_api_snapshot.rs`

### Prerequisites

`cargo-public-api` is a **host tool** — it is intentionally NOT a
workspace dev-dependency (see TASK-AGS-011 spec §Out of Scope). Install
once per workstation:

```bash
cargo install cargo-public-api --locked
rustup toolchain install nightly --profile minimal
```

The nightly toolchain is required because `cargo-public-api` uses
`rustdoc` JSON output, which is nightly-only. Neither the workspace
build nor the normal `cargo test` runs use nightly — only this script
and the two drift tests invoke it.

### Regenerating

```bash
bash scripts/regen-public-api.sh
```

The script:

1. Checks both `cargo-public-api` and a `nightly` toolchain exist.
   Missing either prints a `SKIP:` line and exits 2 — it is **not** a
   hard error so the script can be wired into CI without bricking the
   pipeline on a fresh runner.
2. Captures `cargo public-api --package archon-memory --simplified` into
   `tests/fixtures/baseline/archon_memory_api.txt`.
3. Captures `cargo public-api --package archon-core --simplified`, then
   `grep -F 'archon_core::agents::memory::'` to get only the preserve-D8
   sub-surface, into `tests/fixtures/baseline/agents_memory_api.txt`.
4. Prefixes each file with a single `# cargo-public-api <version>`
   header so tool-version drift is distinguishable from code drift.

The script is deterministic — every cargo invocation pins
`CARGO_BUILD_JOBS=1` (WSL2 safety, see `tests/fixtures/baseline/README.md`)
and the grep filter is a literal substring match sorted by
`cargo public-api`'s own canonical order. Running the script twice at
a fixed git rev produces byte-identical files.

### When to regenerate

ONLY when all three hold:

1. A legitimate, reviewer-approved public-API change has landed.
2. The change is documented in the crate's CHANGELOG (or PR body).
3. You are on `main` or the approved feature branch — **never**
   regenerate to "make CI green" on a speculative branch.

Never regenerate to silence a drift test. The drift is the signal.
