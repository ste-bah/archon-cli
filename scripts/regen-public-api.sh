#!/usr/bin/env bash
# regen-public-api.sh — capture cargo public-api baselines for the
# preserve-D8 crates (archon-memory + archon-core::agents::memory).
#
# Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-011.md
# Based on:  00-prd-analysis.md REQ-FOR-PRESERVE-D8 (d), NFR-ARCH-002
#            (backward compat N-1)
#
# Writes two snapshots into tests/fixtures/baseline/:
#
#   archon_memory_api.txt   full archon-memory public API (~900 lines)
#   agents_memory_api.txt   grep-filtered archon-core public items under
#                           the `agents::memory::` subtree
#
# Each file has a deterministic `# cargo-public-api <version>` header
# on its first line so drift between tool versions is distinguishable
# from real code drift.
#
# Requires:
#   - cargo-public-api on PATH (cargo install cargo-public-api --locked)
#   - a nightly rust toolchain (rustup toolchain install nightly)
#
# Missing either produces a SKIP message and exit code 2.
#
# WSL2 safety: every cargo invocation pins CARGO_BUILD_JOBS=1.

set -euo pipefail

if ! ROOT=$(git rev-parse --show-toplevel 2>/dev/null); then
    echo "ERROR: regen-public-api.sh must be run inside a git checkout" >&2
    exit 1
fi
cd "$ROOT"

if ! command -v cargo-public-api >/dev/null; then
    echo "SKIP: cargo-public-api not installed." >&2
    echo "      Run: cargo install cargo-public-api --locked" >&2
    exit 2
fi

if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    echo "SKIP: nightly toolchain not installed." >&2
    echo "      Run: rustup toolchain install nightly --profile minimal" >&2
    exit 2
fi

TOOL_VERSION=$(cargo-public-api --version | head -1 | tr -d '\r')
HEADER="# ${TOOL_VERSION}"
BASELINE_DIR="tests/fixtures/baseline"
mkdir -p "$BASELINE_DIR"

TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

# --- archon-memory full surface ---
CARGO_BUILD_JOBS=1 cargo public-api --package archon-memory --simplified \
    2>/dev/null > "$TMP"
{
    echo "$HEADER"
    cat "$TMP"
} > "$BASELINE_DIR/archon_memory_api.txt"

# --- archon-core filtered to archon_core::agents::memory::<item> ---
#
# `grep -F` is a literal substring match. The trailing `::` after
# `memory` is a hard boundary — it cannot match `memory_foo::` or
# `memory_tests::` because `_` is not `:`. Empty output (zero lines
# after filter) is tolerated via `|| true`; the resulting snapshot
# is still non-empty because of the `# header` line.
CARGO_BUILD_JOBS=1 cargo public-api --package archon-core --simplified \
    2>/dev/null > "$TMP"
{
    echo "$HEADER"
    grep -F 'archon_core::agents::memory::' "$TMP" || true
} > "$BASELINE_DIR/agents_memory_api.txt"

echo "regen-public-api.sh: wrote snapshots under $BASELINE_DIR/"
wc -l "$BASELINE_DIR/archon_memory_api.txt" \
      "$BASELINE_DIR/agents_memory_api.txt"
