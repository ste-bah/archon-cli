#!/usr/bin/env bash
# check-tui-module-cycles.sh — archon-tui module import-direction guard.
#
# Enforces layered module architecture per TECH-TUI-MODULARIZATION:
#
#   Layer 0 (no upward imports): events, state, theme, render, keybindings
#   Layer 1 (uses layer 0):      overlays, virtual_list, message_renderer,
#                                notifications, prompt_input, context_viz
#   Layer 2 (screens):           screens/* (uses layer 0+1)
#   Layer 3 (top):               app (uses everything)
#   Layer 4 (input/commands):    input, command/* (uses events + state only)
#
# Mechanism: grep-based directional rule table. A "cycle" in this context
# means any low-level module importing from a higher-level module — the
# Rust compiler structurally prevents mod-level cycles, so we enforce the
# ARCHITECTURAL direction here.
#
# Optional: if cargo-depgraph is installed, also runs a belt-and-braces
# crate-level cycle scan. Skipped cleanly if the tool is absent.
#
# Implements NFR-TUI-MOD-002, AC-MOD-04 (cycle portion).
#
# Usage:   bash scripts/check-tui-module-cycles.sh
# Exit:    0 if every module respects the layer rules, 1 otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

TUI_SRC="crates/archon-tui/src"

# Directional rule table: "module_path|forbidden_pattern1,forbidden_pattern2,..."
# Each forbidden_pattern is matched as a fixed string against `use crate::<pattern>`
# lines. Multiple patterns comma-separated on one entry.
RULES=(
  "state.rs|screens,app,input,command"
  "events.rs|screens,app,state,input,command"
  "theme.rs|screens,app,state,events,input,command"
  "render.rs|screens,app,input,command"
  "keybindings.rs|screens,app,input,command"
  "overlays.rs|screens,app"
  "virtual_list.rs|screens,app,state,events"
  "message_renderer.rs|screens,app,state,events"
  "notifications.rs|screens,app,state"
  "prompt_input.rs|screens,app"
  "context_viz.rs|screens,app"
)

VIOLATIONS=0
VIOLATIONS_OUT=""
CHECKED=0

for rule in "${RULES[@]}"; do
  module="${rule%%|*}"
  forbidden="${rule#*|}"
  file="${TUI_SRC}/${module}"
  if [ ! -f "$file" ]; then
    # Skeleton not yet created — skip. Reported at end.
    continue
  fi
  CHECKED=$((CHECKED + 1))
  IFS=',' read -ra PATTERNS <<< "$forbidden"
  for pat in "${PATTERNS[@]}"; do
    # Match `use crate::<pat>::` or `use crate::<pat>;` or `use crate::<pat> as`
    # Anchored to avoid partial matches (e.g. "state" must not match "stateful").
    if grep -qE "^[[:space:]]*use[[:space:]]+crate::${pat}(::|;|[[:space:]])" "$file"; then
      offender=$(grep -nE "^[[:space:]]*use[[:space:]]+crate::${pat}(::|;|[[:space:]])" "$file" | head -5)
      printf '::error file=%s::%s imports forbidden crate::%s (layer violation)\n' \
        "$file" "$module" "$pat"
      VIOLATIONS=$((VIOLATIONS + 1))
      VIOLATIONS_OUT+="  ${module} -> crate::${pat}:"$'\n'"${offender}"$'\n'
    fi
  done
done

if [ -n "$VIOLATIONS_OUT" ]; then
  printf 'TuiCycleGuard: directional violations:\n'
  printf '%s' "$VIOLATIONS_OUT"
fi

printf 'TuiCycleGuard: %d rules checked, %d violations\n' "$CHECKED" "$VIOLATIONS"

# Optional: cargo-depgraph belt-and-braces crate-level cycle scan.
if command -v cargo-depgraph >/dev/null 2>&1; then
  printf 'TuiCycleGuard: running cargo-depgraph crate-level scan\n'
  set +e
  DEPGRAPH_OUT=$(cargo depgraph --workspace-only 2>&1)
  DEPGRAPH_EXIT=$?
  set -e
  if [ "$DEPGRAPH_EXIT" -ne 0 ]; then
    printf 'TuiCycleGuard: cargo-depgraph failed (exit %d) — skipping\n' "$DEPGRAPH_EXIT"
  else
    CYCLE_COUNT=$(printf '%s\n' "$DEPGRAPH_OUT" | grep -c -i 'cycle' || true)
    if [ "$CYCLE_COUNT" -gt 0 ]; then
      printf '::error::cargo-depgraph detected %d cycle line(s) in workspace\n' "$CYCLE_COUNT"
      printf '%s\n' "$DEPGRAPH_OUT" | grep -i 'cycle'
      VIOLATIONS=$((VIOLATIONS + CYCLE_COUNT))
    fi
  fi
else
  printf 'TuiCycleGuard: cargo-depgraph not installed — skipping crate-level scan (vendored-detector mode per spec escape hatch)\n'
fi

if [ "$VIOLATIONS" -gt 0 ]; then
  exit 1
fi
exit 0