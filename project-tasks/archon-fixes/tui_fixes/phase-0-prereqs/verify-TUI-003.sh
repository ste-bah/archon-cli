#!/usr/bin/env bash
# Reference: project-tasks/archon-fixes/tui_fixes/phase-0-prereqs/TASK-TUI-003.md
set -euo pipefail

cd /home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_file_exists() {
    local path="$1"
    [[ -f "$path" ]] || fail "missing file: $path"
}

assert_executable() {
    local path="$1"
    [[ -x "$path" ]] || fail "not executable: $path"
}

# 3. Assert scripts exist and are executable
assert_file_exists "scripts/tui-file-size-gate.sh"
assert_executable "scripts/tui-file-size-gate.sh"
assert_file_exists "scripts/tui-file-size-gate.selftest.sh"
assert_executable "scripts/tui-file-size-gate.selftest.sh"

# 4. Assert fixture files exist
assert_file_exists "tests/fixtures/tui-file-size-gate/pass/src/foo.rs"
assert_file_exists "tests/fixtures/tui-file-size-gate/pass/file-size-allowlist.json"
assert_file_exists "tests/fixtures/tui-file-size-gate/fail/src/bad.rs"
assert_file_exists "tests/fixtures/tui-file-size-gate/fail/file-size-allowlist.json"

# 5. Assert pass fixture foo.rs < 500 lines
pass_lines=$(wc -l < "tests/fixtures/tui-file-size-gate/pass/src/foo.rs")
if [[ "$pass_lines" -ge 500 ]]; then
    fail "pass fixture foo.rs should have < 500 lines, got $pass_lines"
fi

# 6. Assert fail fixture bad.rs > 500 lines
fail_lines=$(wc -l < "tests/fixtures/tui-file-size-gate/fail/src/bad.rs")
if [[ "$fail_lines" -le 500 ]]; then
    fail "fail fixture bad.rs should have > 500 lines, got $fail_lines"
fi

# 7. Run selftest and assert exit 0
if ! bash scripts/tui-file-size-gate.selftest.sh; then
    fail "selftest script did not exit 0"
fi

# 8. Run real gate against real allowlist and assert exit 0
if ! bash scripts/tui-file-size-gate.sh; then
    fail "real gate script did not exit 0 against real allowlist"
fi

# 9. Assert ratchet behaviour: override to 1 line must fail
set +e
ALLOWLIST_OVERRIDE_LINES=1 bash scripts/tui-file-size-gate.sh >/dev/null 2>&1
ratchet_rc=$?
set -e
if [[ "$ratchet_rc" -eq 0 ]]; then
    fail "ratchet check: expected non-zero with ALLOWLIST_OVERRIDE_LINES=1, got 0"
fi

# 10. Assert no .rs source files were modified
diff_out=$(git diff --name-only src/ crates/archon-tui/src/ 2>/dev/null || true)
if [[ -n "$diff_out" ]]; then
    fail "unexpected modifications to source files:\n$diff_out"
fi

# 11. Success
echo "OK: verify-TUI-003 passed"
