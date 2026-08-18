#!/usr/bin/env bash
#
# scripts/check-deferral-markers.sh — cap the deferral markers in the tree.
#
# # Why this exists
#
# `scripts/tests/` used to hold two dozen per-task shell gates, each of which
# grepped a hardcoded file path for a hardcoded `TODO(TASK-x)` string to prove
# some slice was no longer stubbed. Every one of them was deleted, because the
# rest of what they checked — "does `pub fn draw_skills_menu` exist", "does
# `render/mod.rs` mention it" — is already enforced by the compiler, and the
# path-hardcoding meant they went red on file splits rather than on defects.
# All eight that were failing when they were removed were false alarms caused
# by a file being split for the 500-line FileSizeGuard.
#
# One thing those gates checked is NOT compiler-enforceable, and it is the one
# that matters: a slice that was wired can quietly regress to a stub, and the
# marker left behind is the only trace. So that check survives, once, here,
# over the whole tree rather than per-task over one path.
#
# # The rule
#
# A deferral marker — `TODO(TICKET)`, `FIXME(TICKET)`, or a `Deferred (TICKET)`
# doc heading — is allowed only if it is listed in
# `scripts/check-deferral-markers.allowlist` with the issue that tracks it.
# Adding one means editing the allowlist, which means it is reviewed. Fixing
# the underlying deferral means deleting its allowlist line, and the gate then
# reddens if the marker comes back.
#
# Lowercase `TODO(someone)` and bare `TODO:` are deliberately NOT matched: this
# is about tracked, ticketed deferrals, not ordinary notes.
#
# Usage:
#   bash scripts/check-deferral-markers.sh              # scan the tree
#   bash scripts/check-deferral-markers.sh --self-test  # prove it can go red
#
# Exits 0 GREEN, 1 RED.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ALLOWLIST="$SCRIPT_DIR/check-deferral-markers.allowlist"

# A ticket is an uppercase prefix, a dash, then anything wordish:
# TUI-620-followup, TASK-AGS-801, REQ-RESEARCH-007, P0B-3.
TICKET='[A-Z][A-Z0-9]*-[A-Za-z0-9._-]+'
PATTERN="(TODO|FIXME)\\(${TICKET}\\)|[Dd]eferred \\(${TICKET}\\)"

# Source roots. Naming them explicitly rather than walking `.` is not an
# optimisation: `target/` holds vendored crate sources and build output that
# dwarf the tree, so scanning from the root both takes minutes and reports
# other people's markers as ours.
SOURCE_ROOTS=(src crates)

# Print `path:line:marker` for every marker under $1.
scan() {
    local root="$1"
    local -a present=()
    for dir in "${SOURCE_ROOTS[@]}"; do
        [[ -d "$root/$dir" ]] && present+=("$dir")
    done
    [[ ${#present[@]} -gt 0 ]] || return 0
    ( cd "$root" 2>/dev/null || return 0
      grep -rnoE "$PATTERN" --include='*.rs' "${present[@]}" 2>/dev/null )
}

# ---------------------------------------------------------------------
# --self-test: a gate nobody has watched go red is indistinguishable
# from a gate that cannot. Plant a marker in a synthetic tree and require
# the same scan() the real run uses to find it.
# ---------------------------------------------------------------------
if [[ "${1:-}" == "--self-test" ]]; then
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    mkdir -p "$TMP/src"
    cat >"$TMP/src/planted.rs" <<'EOF'
//! # Deferred (SELFTEST-001)
// TODO(SELFTEST-002): this line must be found
// TODO(lowercase-name): this line must NOT be found
// TODO: a bare note must NOT be found
EOF
    FOUND="$(scan "$TMP")"
    rc=0
    if ! grep -q 'SELFTEST-001' <<<"$FOUND"; then
        echo "RED(self-test): the Deferred (TICKET) heading form was not detected"
        rc=1
    fi
    if ! grep -q 'SELFTEST-002' <<<"$FOUND"; then
        echo "RED(self-test): the TODO(TICKET) form was not detected"
        rc=1
    fi
    if grep -q 'lowercase-name' <<<"$FOUND"; then
        echo "RED(self-test): lowercase TODO(name) was matched; pattern is too broad"
        rc=1
    fi
    if [[ "$(grep -c . <<<"$FOUND")" -ne 2 ]]; then
        echo "RED(self-test): expected exactly 2 hits, got:"
        echo "$FOUND"
        rc=1
    fi
    if [[ "$rc" -eq 0 ]]; then
        echo "GREEN(self-test): the matcher detects planted markers and ignores untracked notes"
    fi
    exit "$rc"
fi

# ---------------------------------------------------------------------
# Real run.
# ---------------------------------------------------------------------
if [[ ! -f "$ALLOWLIST" ]]; then
    echo "RED: allowlist missing: $ALLOWLIST"
    exit 1
fi

# Allowlist entries are `path:MARKER`, one per line. `#` comments and blank
# lines are ignored. A marker is allowed only at the path it is listed for —
# copying a deferral into a second file has to be reviewed too.
mapfile -t ALLOWED < <(grep -vE '^\s*(#|$)' "$ALLOWLIST" | sed 's/[[:space:]]*$//')

FAIL=0
UNMATCHED=("${ALLOWED[@]}")

while IFS= read -r hit; do
    [[ -n "$hit" ]] || continue
    # hit is path:line:marker — drop the line number for allowlist matching.
    path="${hit%%:*}"
    marker="${hit##*:}"
    key="$path:$marker"

    allowed=0
    for entry in "${ALLOWED[@]}"; do
        if [[ "$entry" == "$key" ]]; then
            allowed=1
            # Mark this allowlist entry as still needed.
            for i in "${!UNMATCHED[@]}"; do
                [[ "${UNMATCHED[$i]}" == "$entry" ]] && unset 'UNMATCHED[i]'
            done
            break
        fi
    done

    if [[ "$allowed" -eq 0 ]]; then
        echo "RED: unallowlisted deferral marker: $hit"
        echo "     If this is real, add '$key' to scripts/check-deferral-markers.allowlist"
        echo "     with the issue that tracks it. If the work is done, delete the marker."
        FAIL=1
    fi
done < <(scan "$REPO_ROOT")

# A stale allowlist is how a cap stops being a cap: the entry stays, the marker
# goes, and the next deferral slides in under a line that was already spent.
for entry in "${UNMATCHED[@]:-}"; do
    [[ -n "$entry" ]] || continue
    echo "RED: stale allowlist entry (no such marker in the tree): $entry"
    echo "     The deferral was resolved — delete this line."
    FAIL=1
done

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check-deferral-markers: FAILED"
    exit 1
fi

echo "GREEN: every deferral marker is allowlisted, and every allowlist entry is live (${#ALLOWED[@]} tracked)"
