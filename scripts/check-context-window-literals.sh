#!/usr/bin/env bash
#
# check-context-window-literals.sh — no hardcoded context windows in production code.
#
# A context window belongs to the model, so a literal in Rust is a value that
# goes stale silently: the number stays 200k while the model it describes moves,
# and nothing fails. Same for the 4/5 budget fraction, which has to be derived
# from the window rather than re-typed beside it.
#
# Usage:   bash scripts/check-context-window-literals.sh
# Exit:    0 if no production hit, 1 otherwise.
set -uo pipefail

# Both searches below swallow rg's stderr, so a missing rg would produce no
# hits and the gate would report "clean" — passing because it never looked.
# Now that this runs in CI (#132) that fail-open is worth one explicit check.
if ! command -v rg >/dev/null 2>&1; then
  echo "check-context-window-literals: ripgrep (rg) not found; refusing to report clean" >&2
  exit 2
fi

# Test code is allowed to name a window, because a fixture asserting on a
# specific model's budget has to state that model's number.
#
# The per-line `grep -v` below cannot express that on its own. This repo marks a
# test module at its *include site* — `#[cfg(test)] mod foo_tests;` in the
# parent — so the attribute is never in the file the literal lives in, and a
# line filter has nothing to match. Roughly 350 files under `src/` are included
# that way. The glob is therefore the mechanism that has to carry it, and it
# covers both conventions: a `tests/` directory, and the `*_test.rs` /
# `*_tests.rs` / `*_tests_<suffix>.rs` naming used for `#[cfg(test)]` modules.
#
# The line filter stays for the case it does handle: an inline `#[cfg(test)]
# mod tests` inside an otherwise-production file.
TEST_GLOBS=(
  --glob '!**/tests/**'
  --glob '!**/*_test.rs'
  --glob '!**/*_tests.rs'
  --glob '!**/*_tests_*.rs'
  --glob '!**/*_test_fixture.rs'
)

literal_hits=$(
  rg "200_000|200000" crates src -g '*.rs' "${TEST_GLOBS[@]}" 2>/dev/null \
    | grep -v "#\\[cfg(test\\|#\\[tokio::test\\|audit-allow" \
    | grep "context_window\\|context_limit\\|window.*200" || true
)
budget_hits=$(
  rg "model_context_window\\s*\\*\\s*4\\s*/\\s*5|context_window\\s*\\*\\s*4\\s*/\\s*5|4\\.0\\s*/\\s*5\\.0" \
    crates src -g '*.rs' "${TEST_GLOBS[@]}" 2>/dev/null \
    | grep -v "#\\[cfg(test\\|#\\[tokio::test\\|audit-allow" || true
)

hits=$(printf '%s\n%s\n' "$literal_hits" "$budget_hits" | sed '/^$/d')

if [ -n "$hits" ]; then
  printf '%s\n' "$hits"
  echo "check-context-window-literals: hardcoded context-window literal found" >&2
  exit 1
fi

echo "check-context-window-literals: clean"
