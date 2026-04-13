#!/usr/bin/env bash
# test_check_file_sizes.sh — TASK-AGS-627
#
# Hermetic test runner for scripts/check-file-sizes.sh (TASK-AGS-002) and
# scripts/install-hooks.sh (TASK-AGS-627).
#
# Each test case runs in its own mktemp -d sandbox. The real check-file-sizes.sh
# is COPIED into each sandbox (along with a minimal allowlist) so the tests are
# fully hermetic and never touch the real tree.
#
# Usage: bash scripts/tests/test_check_file_sizes.sh
# Exit:  0 iff all 11 sandbox/helper tests pass. Test (l) is informational only.

set -uo pipefail

# Resolve real script locations (relative to this test file).
TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="$(cd "$TEST_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPTS_DIR/.." && pwd)"

REAL_CHECK_SCRIPT="$SCRIPTS_DIR/check-file-sizes.sh"
REAL_INSTALL_SCRIPT="$SCRIPTS_DIR/install-hooks.sh"

PASS_COUNT=0
FAIL_COUNT=0
FAILED_TESTS=()

# Color helpers (only when stdout is a tty).
if [ -t 1 ]; then
  C_GREEN=$'\033[32m'
  C_RED=$'\033[31m'
  C_YELLOW=$'\033[33m'
  C_RESET=$'\033[0m'
else
  C_GREEN=""
  C_RED=""
  C_YELLOW=""
  C_RESET=""
fi

pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  printf '%sPASS%s  %s\n' "$C_GREEN" "$C_RESET" "$1"
}

fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  FAILED_TESTS+=("$1")
  printf '%sFAIL%s  %s\n' "$C_RED" "$C_RESET" "$1"
  if [ "${2-}" != "" ]; then
    printf '      %s\n' "$2"
  fi
}

info() {
  printf '%sINFO%s  %s\n' "$C_YELLOW" "$C_RESET" "$1"
}

# Make a sandbox repo. Echoes the absolute path. Caller is responsible for cleanup.
make_sandbox() {
  local sb
  sb="$(mktemp -d)"
  mkdir -p "$sb/scripts"
  # Copy the real check script into the sandbox so the test is hermetic.
  cp "$REAL_CHECK_SCRIPT" "$sb/scripts/check-file-sizes.sh"
  chmod +x "$sb/scripts/check-file-sizes.sh"
  # Empty allowlist by default (caller can overwrite).
  : > "$sb/scripts/check-file-sizes.allowlist"
  printf '%s' "$sb"
}

# Generate a file with N lines of dummy rust content.
make_lines() {
  local path="$1"
  local n="$2"
  mkdir -p "$(dirname "$path")"
  local i=1
  : > "$path"
  while [ "$i" -le "$n" ]; do
    printf '// line %d\n' "$i" >> "$path"
    i=$((i + 1))
  done
}

# Run check-file-sizes.sh in a sandbox; capture exit + stdout.
run_check_in() {
  local sb="$1"
  ( cd "$sb" && bash scripts/check-file-sizes.sh ) 2>&1
}

# ============================================================================
# Test (a): passes_on_clean_sandbox
# ============================================================================
test_passes_on_clean_sandbox() {
  local name="a. passes_on_clean_sandbox"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/src/a.rs" 10
  make_lines "$sb/crates/archon-core/src/b.rs" 10
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q '0 over 500'; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (b): fails_on_uncaught_offender
# ============================================================================
test_fails_on_uncaught_offender() {
  local name="b. fails_on_uncaught_offender"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/src/big.rs" 600
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -ne 0 ] \
     && printf '%s' "$out" | grep -q 'offenders (> 500 lines)' \
     && printf '%s' "$out" | grep -q 'big.rs'; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (c): passes_when_offender_is_allowlisted
# ============================================================================
test_passes_when_offender_is_allowlisted() {
  local name="c. passes_when_offender_is_allowlisted"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/src/big.rs" 600
  printf 'src/big.rs\n' > "$sb/scripts/check-file-sizes.allowlist"
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ] \
     && printf '%s' "$out" | grep -q 'allowlisted' \
     && printf '%s' "$out" | grep -q 'big.rs'; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (d): ignores_target_directory
# ============================================================================
test_ignores_target_directory() {
  local name="d. ignores_target_directory"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/target/bloat.rs" 900
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ]; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (e): ignores_cargo_cache
# ============================================================================
test_ignores_cargo_cache() {
  local name="e. ignores_cargo_cache"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/.cargo/bloat.rs" 900
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ]; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (f): ignores_tests_fixtures
# ============================================================================
test_ignores_tests_fixtures() {
  local name="f. ignores_tests_fixtures"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/tests/fixtures/bloat.rs" 900
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ]; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (g): ignores_non_rust_files
# ============================================================================
test_ignores_non_rust_files() {
  local name="g. ignores_non_rust_files"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/src/huge.txt" 1000
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q '0 files checked'; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (h): allowlist_comments_and_blanks_ignored
# ============================================================================
test_allowlist_comments_and_blanks_ignored() {
  local name="h. allowlist_comments_and_blanks_ignored"
  local sb
  sb="$(make_sandbox)"
  make_lines "$sb/src/ok.rs" 600
  cat > "$sb/scripts/check-file-sizes.allowlist" <<'EOF'
# header comment

src/ok.rs
# trailing comment
EOF
  local out rc
  out="$(run_check_in "$sb")"
  rc=$?
  if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'allowlisted'; then
    pass "$name"
  else
    fail "$name" "rc=$rc out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Helper: build a sandbox git repo that has both check-file-sizes.sh AND
# install-hooks.sh copied in, and is `git init`'d.
# ============================================================================
make_git_sandbox() {
  local sb
  sb="$(mktemp -d)"
  mkdir -p "$sb/scripts"
  cp "$REAL_CHECK_SCRIPT" "$sb/scripts/check-file-sizes.sh"
  chmod +x "$sb/scripts/check-file-sizes.sh"
  : > "$sb/scripts/check-file-sizes.allowlist"
  if [ -f "$REAL_INSTALL_SCRIPT" ]; then
    cp "$REAL_INSTALL_SCRIPT" "$sb/scripts/install-hooks.sh"
    chmod +x "$sb/scripts/install-hooks.sh"
  fi
  ( cd "$sb" && git init -q && git config user.email "t@t.t" && git config user.name "t" ) >/dev/null 2>&1
  printf '%s' "$sb"
}

# ============================================================================
# Test (i): install_hooks_fresh
# ============================================================================
test_install_hooks_fresh() {
  local name="i. install_hooks_fresh"
  if [ ! -f "$REAL_INSTALL_SCRIPT" ]; then
    fail "$name" "scripts/install-hooks.sh does not exist yet"
    return
  fi
  local sb
  sb="$(make_git_sandbox)"
  local out rc
  out="$( cd "$sb" && bash scripts/install-hooks.sh 2>&1 )"
  rc=$?
  local git_common_dir
  git_common_dir="$( cd "$sb" && git rev-parse --git-common-dir 2>/dev/null )"
  if [ -z "$git_common_dir" ]; then
    fail "$name" "could not resolve git-common-dir in sandbox"
    rm -rf "$sb"
    return
  fi
  # Resolve relative git-common-dir against sandbox.
  case "$git_common_dir" in
    /*) : ;;
    *)  git_common_dir="$sb/$git_common_dir" ;;
  esac
  local hook="$git_common_dir/hooks/pre-commit"
  if [ "$rc" -eq 0 ] \
     && [ -f "$hook" ] \
     && [ -x "$hook" ] \
     && head -c 2 "$hook" | grep -q '#!' \
     && grep -q 'check-file-sizes.sh' "$hook"; then
    pass "$name"
  else
    fail "$name" "rc=$rc hook=$hook exists=$( [ -f "$hook" ] && echo y || echo n ) out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (j): install_hooks_refuses_overwrite_without_force
# ============================================================================
test_install_hooks_refuses_overwrite_without_force() {
  local name="j. install_hooks_refuses_overwrite_without_force"
  if [ ! -f "$REAL_INSTALL_SCRIPT" ]; then
    fail "$name" "scripts/install-hooks.sh does not exist yet"
    return
  fi
  local sb
  sb="$(make_git_sandbox)"
  local git_common_dir
  git_common_dir="$( cd "$sb" && git rev-parse --git-common-dir 2>/dev/null )"
  case "$git_common_dir" in
    /*) : ;;
    *)  git_common_dir="$sb/$git_common_dir" ;;
  esac
  mkdir -p "$git_common_dir/hooks"
  local hook="$git_common_dir/hooks/pre-commit"
  printf '#!/bin/sh\necho custom\n' > "$hook"
  chmod +x "$hook"
  local before_sum
  before_sum="$(sha256sum "$hook" | awk '{print $1}')"
  local out rc
  out="$( cd "$sb" && bash scripts/install-hooks.sh 2>&1 )"
  rc=$?
  local after_sum
  after_sum="$(sha256sum "$hook" | awk '{print $1}')"
  if [ "$rc" -ne 0 ] \
     && [ "$before_sum" = "$after_sum" ] \
     && printf '%s' "$out" | grep -qi 'warn\|exist\|refus'; then
    pass "$name"
  else
    fail "$name" "rc=$rc before=$before_sum after=$after_sum out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (k): install_hooks_force_overwrites_with_backup
# ============================================================================
test_install_hooks_force_overwrites_with_backup() {
  local name="k. install_hooks_force_overwrites_with_backup"
  if [ ! -f "$REAL_INSTALL_SCRIPT" ]; then
    fail "$name" "scripts/install-hooks.sh does not exist yet"
    return
  fi
  local sb
  sb="$(make_git_sandbox)"
  local git_common_dir
  git_common_dir="$( cd "$sb" && git rev-parse --git-common-dir 2>/dev/null )"
  case "$git_common_dir" in
    /*) : ;;
    *)  git_common_dir="$sb/$git_common_dir" ;;
  esac
  mkdir -p "$git_common_dir/hooks"
  local hook="$git_common_dir/hooks/pre-commit"
  local custom_marker='echo custom-marker-xyzzy'
  printf '#!/bin/sh\n%s\n' "$custom_marker" > "$hook"
  chmod +x "$hook"
  local out rc
  out="$( cd "$sb" && bash scripts/install-hooks.sh --force 2>&1 )"
  rc=$?
  # Find the backup file.
  local backup
  backup="$(ls "$git_common_dir/hooks/" 2>/dev/null | grep '^pre-commit\.bak\.' | head -n1 || true)"
  local backup_path=""
  if [ -n "$backup" ]; then
    backup_path="$git_common_dir/hooks/$backup"
  fi
  if [ "$rc" -eq 0 ] \
     && [ -n "$backup_path" ] \
     && [ -f "$backup_path" ] \
     && grep -q 'custom-marker-xyzzy' "$backup_path" \
     && grep -q 'check-file-sizes.sh' "$hook"; then
    pass "$name"
  else
    fail "$name" "rc=$rc backup=$backup_path out=$out"
  fi
  rm -rf "$sb"
}

# ============================================================================
# Test (l): real_tree_status (informational only)
# ============================================================================
test_real_tree_status() {
  local name="l. real_tree_status (informational)"
  local out rc
  out="$( cd "$REPO_ROOT" && bash scripts/check-file-sizes.sh 2>&1 )"
  rc=$?
  if [ "$rc" -eq 0 ]; then
    info "$name -- REAL TREE STATUS: OK (exit=0)"
  else
    info "$name -- REAL TREE STATUS: FAIL (exit=$rc)"
  fi
  # Print the trailer line for visibility.
  local trailer
  trailer="$(printf '%s' "$out" | grep '^FileSizeGuard:' | tail -n1 || true)"
  if [ -n "$trailer" ]; then
    printf '      %s\n' "$trailer"
  fi
  # Always return success: this is informational.
  return 0
}

# ============================================================================
# Run everything
# ============================================================================
main() {
  printf 'Running TASK-AGS-627 file-size guard tests...\n\n'

  test_passes_on_clean_sandbox
  test_fails_on_uncaught_offender
  test_passes_when_offender_is_allowlisted
  test_ignores_target_directory
  test_ignores_cargo_cache
  test_ignores_tests_fixtures
  test_ignores_non_rust_files
  test_allowlist_comments_and_blanks_ignored
  test_install_hooks_fresh
  test_install_hooks_refuses_overwrite_without_force
  test_install_hooks_force_overwrites_with_backup
  test_real_tree_status

  printf '\n'
  printf '%d passed, %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"
  if [ "$FAIL_COUNT" -gt 0 ]; then
    printf 'Failed tests:\n'
    for t in "${FAILED_TESTS[@]}"; do
      printf '  - %s\n' "$t"
    done
    exit 1
  fi
  exit 0
}

main "$@"
