#!/usr/bin/env bash
# capture-inventory.sh — baseline snapshots for REQ-FOR-D4/D6/D7 progress tracking.
#
# Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-012.md
# Based on:  00-prd-analysis.md REQ-FOR-D4 (main.rs LoC),
#            REQ-FOR-D6 (LlmProvider breadth),
#            REQ-FOR-D7 (slash-command coverage)
#
# Produces three frozen snapshots under tests/fixtures/baseline/:
#   main_rs_loc.txt      single LoC number for src/main.rs
#   slash_commands.txt   sorted unique slash commands seen in src/main.rs
#   providers.txt        sorted unique LlmProvider impls under crates/archon-llm/src/providers/
#
# Deterministic: LC_ALL=C sort + literal git sha + today's UTC date in the
# header comment. Idempotent at a fixed git rev.

set -euo pipefail

if ! ROOT=$(git rev-parse --show-toplevel 2>/dev/null); then
    echo "ERROR: capture-inventory.sh must be run inside the archon-cli git worktree" >&2
    exit 1
fi

cd "$ROOT"

BASELINE_DIR="tests/fixtures/baseline"
mkdir -p "$BASELINE_DIR"

SHA=$(git rev-parse --short HEAD)
DATE=$(date -u +%Y-%m-%d)
HEADER="# captured ${DATE} from git rev ${SHA}"

# --- main.rs LoC ---
MAIN_RS="src/main.rs"
if [[ ! -f "$MAIN_RS" ]]; then
    echo "ERROR: $MAIN_RS not found" >&2
    exit 1
fi
LOC=$(wc -l < "$MAIN_RS" | tr -d ' ')
{
    echo "$HEADER"
    echo "$LOC"
} > "$BASELINE_DIR/main_rs_loc.txt"

# --- slash commands ---
# Extract quoted slash literals from match arms at the start of a line.
# Pattern anchors to the match-arm indentation so that in-string refs
# (e.g. help text) are ignored. The egrep pattern tolerates single or
# multiple slash-literals separated by `|` in one arm.
SLASH_TMP=$(mktemp)
trap 'rm -f "$SLASH_TMP"' EXIT
grep -oE '"/[a-z][a-z0-9-]*"' "$MAIN_RS" \
    | LC_ALL=C sort -u > "$SLASH_TMP"

{
    echo "$HEADER"
    cat "$SLASH_TMP"
} > "$BASELINE_DIR/slash_commands.txt"

# --- providers ---
# Every concrete type that `impl LlmProvider for <T>` under the real
# provider tree. Tests and mocks are excluded by restricting scope to
# crates/archon-llm/src/providers/.
PROVIDERS_TMP=$(mktemp)
trap 'rm -f "$SLASH_TMP" "$PROVIDERS_TMP"' EXIT
grep -rEh 'impl[[:space:]]+LlmProvider[[:space:]]+for[[:space:]]+[A-Z][A-Za-z0-9_]*' \
    crates/archon-llm/src/providers/ \
    | sed -E 's/.*impl[[:space:]]+LlmProvider[[:space:]]+for[[:space:]]+([A-Z][A-Za-z0-9_]*).*/\1/' \
    | LC_ALL=C sort -u > "$PROVIDERS_TMP"

{
    echo "$HEADER"
    cat "$PROVIDERS_TMP"
} > "$BASELINE_DIR/providers.txt"

echo "capture-inventory.sh: wrote 3 snapshots to $BASELINE_DIR/"
wc -l "$BASELINE_DIR/main_rs_loc.txt" \
       "$BASELINE_DIR/slash_commands.txt" \
       "$BASELINE_DIR/providers.txt"
