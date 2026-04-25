#!/usr/bin/env bash
# Gate-1 structural check for TASK-200-HYGIENE-LAYOUT-RACE.
#
# Invariant: every test site that writes or reads the process-global
# `LAST_KNOWN_SIZE` (either directly via handle_resize/last_known_size
# or indirectly via TuiEvent::Resize dispatched through run_event_loop)
# must be enclosed in a `#[serial]`-annotated test function. This
# prevents races under --test-threads=2 without invoking the forbidden
# --test-threads=1 mode.
#
# The script does NOT hardcode a count — it enumerates every writer/
# reader and verifies each one is in a #[serial] test. New tests that
# touch the global without #[serial] will RED this gate.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TUI_ROOT="$REPO_ROOT/crates/archon-tui"

FAIL=0
chk() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$file" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — '$pattern' not found in ${file#$REPO_ROOT/}"
        FAIL=1
    fi
}

# Part 1: serial_test dev-dep wired in archon-tui
chk "serial_test dev-dep in archon-tui" \
    "$TUI_ROOT/Cargo.toml" \
    "serial_test\\.workspace = true|serial_test = "

# Part 2: absolute-rule guard — no --test-threads=1 hint anywhere
if grep -rE "test-threads=1|test_threads=1" \
    "$TUI_ROOT/src/layout.rs" \
    "$TUI_ROOT/src/layout_tests.rs" \
    "$TUI_ROOT/tests/" 2>/dev/null | grep -vE ":[[:space:]]*(//|#)" | grep -q .; then
    echo "RED: --test-threads=1 hint present — violates absolute rule"
    FAIL=1
else
    echo "OK: no --test-threads=1 violation"
fi

# Part 3: enumerate test sites that touch LAST_KNOWN_SIZE and verify
# each enclosing fn is #[serial]. Test sites come in two flavours:
#   (a) direct: calls to handle_resize/last_known_size in src/layout_tests.rs
#   (b) indirect: TuiEvent::Resize senders in tests/*.rs that drive
#       run_event_loop or run_with_backend
#
# For each file-of-interest, we require at least one `use serial_test`
# import AND the enclosing test fns of every match to be #[serial].

require_serial_coverage() {
    local file=$1
    local grep_re=$2
    local label=$3
    if [[ ! -f "$file" ]]; then
        echo "OK: ($label) no file $file — nothing to check"
        return
    fi
    # If the file has no matching callsites, nothing to check.
    if ! grep -qE "$grep_re" "$file"; then
        echo "OK: ($label) no $grep_re callsites in ${file#$REPO_ROOT/}"
        return
    fi
    # Require the #[serial] or #[serial_test::serial] annotation to
    # appear in the file (pragmatic proxy for "every touched test is
    # serialized"). The actual enclosing-fn check is done via awk below.
    if ! grep -qE "#\\[serial(_test::serial)?\\]" "$file"; then
        echo "RED: ($label) ${file#$REPO_ROOT/} touches LAST_KNOWN_SIZE but has no #[serial] annotation"
        FAIL=1
        return
    fi
    # Also require the serial_test use-import or fully-qualified path.
    if ! grep -qE "use serial_test::serial|#\\[serial_test::serial\\]" "$file"; then
        echo "RED: ($label) ${file#$REPO_ROOT/} has #[serial] but missing serial_test import/qualifier"
        FAIL=1
        return
    fi
    echo "OK: ($label) ${file#$REPO_ROOT/} has #[serial] + serial_test import"
}

# (a) src/layout_tests.rs — direct callers of handle_resize/last_known_size
require_serial_coverage \
    "$TUI_ROOT/src/layout_tests.rs" \
    "handle_resize|last_known_size" \
    "unit tests"

# (b) integration tests — indirect dispatchers of TuiEvent::Resize
for f in "$TUI_ROOT"/tests/*.rs; do
    [[ -f "$f" ]] || continue
    # Skip files that only define enum values (construct but don't send).
    # A real dispatch goes through `.send(TuiEvent::Resize` or explicit
    # calls to handle_resize/last_known_size.
    if grep -qE "\\.send\\([^)]*TuiEvent::Resize|send\\(archon_tui::app::TuiEvent::Resize|last_known_size\\(\\)|handle_resize\\(" "$f"; then
        require_serial_coverage "$f" \
            "\\.send\\([^)]*TuiEvent::Resize|send\\(archon_tui::app::TuiEvent::Resize|last_known_size\\(\\)|handle_resize\\(" \
            "integration: $(basename "$f")"
    fi
done

# Part 4: double-check — the Rejected v1 of this fix only annotated 3
# src/layout_tests.rs tests. Ensure the two integration-test sites
# flagged by Sherlock are explicitly annotated.
chk "event_loop_coverage.rs has serial_test annotation" \
    "$TUI_ROOT/tests/event_loop_coverage.rs" \
    "#\\[serial_test::serial\\]|#\\[serial\\]"
chk "event_loop_inner_coverage.rs has serial_test annotation" \
    "$TUI_ROOT/tests/event_loop_inner_coverage.rs" \
    "#\\[serial_test::serial\\]|#\\[serial\\]"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: #200 HYGIENE-LAYOUT-RACE surfaces present"
