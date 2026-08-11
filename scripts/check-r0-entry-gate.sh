#!/usr/bin/env bash
# check-r0-entry-gate.sh
#
# Re-verifies the frozen R0 closure evidence in
# docs/development/r0-entry-gate.evidence against the working tree and git
# history, and prints an explicit PASS/FAIL for the R0 entry gate.
#
# Reference: docs/development/learning-roadmap-r1-r8-w5-w6.md line 35 (R0 entry
# gate) and reports/core-audit-2026-07-11.md findings 9, 11, 17, 40-43.
#
# The roadmap forbids promoting any behaviour-changing slice (R1-R8, W5-W6)
# while R0 is open, and forbids inferring closure from prose. This script is the
# mechanical half of that: the evidence file states what closure means for each
# finding, and this re-checks every statement. A deleted anchor, a reintroduced
# defect signature, a removed behavioural test, a dropped closure commit, or a
# verdict flipped to `open` all turn the gate red.
#
# Usage:
#   ./check-r0-entry-gate.sh                    # verify (commit ancestry best-effort)
#   ./check-r0-entry-gate.sh --require-commits  # verify; shallow clone is a failure
#   ./check-r0-entry-gate.sh --self-test        # prove the gate can go red
#   ./check-r0-entry-gate.sh --manifest F --base-dir D --git-root G
#
# Exit codes: 0 gate PASS, 1 gate FAIL, 2 usage/environment error.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

MANIFEST="$REPO_ROOT/docs/development/r0-entry-gate.evidence"
BASE_DIR="$REPO_ROOT"
GIT_DIR_ROOT="$REPO_ROOT"
REQUIRE_COMMITS=0
SELF_TEST=0
REQUIRED_FINDINGS="9 11 17 40 41 42 43 S1-shadow-containment"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --manifest)        MANIFEST="${2:?--manifest needs a path}"; shift 2 ;;
        --base-dir)        BASE_DIR="${2:?--base-dir needs a path}"; shift 2 ;;
        # Needed because a Windows-created git worktree stores an absolute
        # `C:\...` gitdir that a WSL shell cannot resolve; the checkout CI uses
        # is an ordinary repository and needs no override.
        --git-root)        GIT_DIR_ROOT="${2:?--git-root needs a path}"; shift 2 ;;
        --required)        REQUIRED_FINDINGS="${2:?--required needs a list}"; shift 2 ;;
        --require-commits) REQUIRE_COMMITS=1; shift ;;
        --self-test)       SELF_TEST=1; shift ;;
        -h|--help)         sed -n '1,25p' "$0"; exit 0 ;;
        *)                 echo "ERROR: unknown flag: $1" >&2; exit 2 ;;
    esac
done

FAILED=0
CURRENT=""

fail() {
    local finding="$1" message="$2"
    echo "FAIL [finding $finding] $message"
    FAILED=1
}

# --- directive checks -------------------------------------------------------

resolve() {
    # Echo the absolute path for a manifest-relative path, or empty if missing.
    local rel="$1"
    if [[ -f "$BASE_DIR/$rel" ]]; then
        echo "$BASE_DIR/$rel"
    fi
}

check_anchor() {
    local finding="$1" rel="$2" pattern="$3" kind="$4"
    local path
    path="$(resolve "$rel")"
    if [[ -z "$path" ]]; then
        fail "$finding" "$kind target file is missing: $rel"
        return
    fi
    if ! grep -Eq -- "$pattern" "$path"; then
        fail "$finding" "$kind gone from $rel: /$pattern/"
    fi
}

check_absent() {
    local finding="$1" rel="$2" pattern="$3"
    local path
    path="$(resolve "$rel")"
    if [[ -z "$path" ]]; then
        fail "$finding" "absent-check target file is missing: $rel"
        return
    fi
    if grep -Eq -- "$pattern" "$path"; then
        fail "$finding" "defect signature reintroduced in $rel: /$pattern/"
        grep -nE -- "$pattern" "$path" | sed 's/^/           /'
    fi
}

check_test() {
    local finding="$1" rel="$2" fn="$3" kind="$4"
    local path
    path="$(resolve "$rel")"
    if [[ -z "$path" ]]; then
        fail "$finding" "$kind file is missing: $rel"
        return
    fi
    if ! grep -Eq -- "fn[[:space:]]+${fn}[[:space:]]*[(<]" "$path"; then
        fail "$finding" "$kind removed from $rel: fn $fn"
    fi
}

check_order() {
    # first match of $3 must appear on an earlier line than first match of $4
    local finding="$1" rel="$2" first="$3" second="$4"
    local path line_first line_second
    path="$(resolve "$rel")"
    if [[ -z "$path" ]]; then
        fail "$finding" "order-check target file is missing: $rel"
        return
    fi
    line_first="$(grep -nE -- "$first" "$path" | head -n1 | cut -d: -f1)"
    line_second="$(grep -nE -- "$second" "$path" | head -n1 | cut -d: -f1)"
    if [[ -z "$line_first" ]]; then
        fail "$finding" "order-check first pattern gone from $rel: /$first/"
        return
    fi
    if [[ -z "$line_second" ]]; then
        fail "$finding" "order-check second pattern gone from $rel: /$second/"
        return
    fi
    if (( line_first >= line_second )); then
        fail "$finding" "order violated in $rel: /$first/ (line $line_first) must precede /$second/ (line $line_second)"
    fi
}

GIT_MODE=""

detect_git_mode() {
    if ! command -v git >/dev/null 2>&1; then
        GIT_MODE="nogit"
        return
    fi
    if ! git -C "$GIT_DIR_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
        GIT_MODE="nogit"
        return
    fi
    if [[ "$(git -C "$GIT_DIR_ROOT" rev-parse --is-shallow-repository 2>/dev/null)" == "true" ]]; then
        GIT_MODE="shallow"
        return
    fi
    GIT_MODE="full"
}

check_commit() {
    local finding="$1" sha="$2"
    if [[ ! "$sha" =~ ^[0-9a-f]{40}$ ]]; then
        fail "$finding" "commit reference is not a full 40-hex sha: '$sha'"
        return
    fi
    case "$GIT_MODE" in
        full)
            if ! git -C "$GIT_DIR_ROOT" cat-file -e "${sha}^{commit}" 2>/dev/null; then
                fail "$finding" "closure commit $sha does not exist in this repository"
                return
            fi
            if ! git -C "$GIT_DIR_ROOT" merge-base --is-ancestor "$sha" HEAD 2>/dev/null; then
                fail "$finding" "closure commit $sha is not an ancestor of HEAD; the fix is not in this history"
            fi
            ;;
        shallow|nogit)
            if (( REQUIRE_COMMITS )); then
                fail "$finding" "commit ancestry for $sha is unverifiable here (git mode: $GIT_MODE) and --require-commits was given"
            else
                COMMIT_SKIPS=$((COMMIT_SKIPS + 1))
            fi
            ;;
    esac
}

# --- manifest parsing -------------------------------------------------------

split_two() {
    # "<left> :: <right>" -> sets SPLIT_L / SPLIT_R
    local value="$1"
    SPLIT_L="${value%% :: *}"
    SPLIT_R="${value#* :: }"
    [[ "$SPLIT_L" != "$value" ]]
}

run_checks() {
    local seen_findings="" line key value
    declare -A VERDICT COUNT_COMMIT COUNT_ANCHOR COUNT_WIRED COUNT_TEST COUNT_SOT VERIFICATION ATTESTOR

    if [[ ! -f "$MANIFEST" ]]; then
        echo "FAIL manifest not found: $MANIFEST"
        return 1
    fi

    detect_git_mode
    COMMIT_SKIPS=0

    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        [[ "$line" =~ ^[[:space:]]*# ]] && continue
        [[ -z "${line//[[:space:]]/}" ]] && continue

        if [[ "$line" =~ ^\[finding:([A-Za-z0-9._-]+)\]$ ]]; then
            CURRENT="${BASH_REMATCH[1]}"
            seen_findings+=" $CURRENT"
            VERDICT[$CURRENT]="<unset>"
            VERIFICATION[$CURRENT]="<unset>"
            ATTESTOR[$CURRENT]=""
            COUNT_COMMIT[$CURRENT]=0
            COUNT_ANCHOR[$CURRENT]=0
            COUNT_WIRED[$CURRENT]=0
            COUNT_TEST[$CURRENT]=0
            COUNT_SOT[$CURRENT]=0
            continue
        fi

        if [[ ! "$line" =~ ^([a-z_]+)[[:space:]]=[[:space:]](.*)$ ]]; then
            echo "FAIL unparseable manifest line: $line"
            FAILED=1
            continue
        fi
        key="${BASH_REMATCH[1]}"
        value="${BASH_REMATCH[2]}"

        if [[ -z "$CURRENT" ]]; then
            echo "FAIL directive outside any [finding:...] block: $line"
            FAILED=1
            continue
        fi

        case "$key" in
            title|note|deviation) ;;
            verdict)      VERDICT[$CURRENT]="$value" ;;
            verification) VERIFICATION[$CURRENT]="$value" ;;
            attested_by)  ATTESTOR[$CURRENT]="$value" ;;
            commit)
                COUNT_COMMIT[$CURRENT]=$(( COUNT_COMMIT[$CURRENT] + 1 ))
                check_commit "$CURRENT" "${value%% *}"
                ;;
            anchor)
                if split_two "$value"; then
                    COUNT_ANCHOR[$CURRENT]=$(( COUNT_ANCHOR[$CURRENT] + 1 ))
                    check_anchor "$CURRENT" "$SPLIT_L" "$SPLIT_R" "anchor"
                else
                    fail "$CURRENT" "anchor needs '<path> :: <regex>': $value"
                fi
                ;;
            wired)
                if split_two "$value"; then
                    COUNT_WIRED[$CURRENT]=$(( COUNT_WIRED[$CURRENT] + 1 ))
                    check_anchor "$CURRENT" "$SPLIT_L" "$SPLIT_R" "live call site"
                else
                    fail "$CURRENT" "wired needs '<path> :: <regex>': $value"
                fi
                ;;
            absent)
                if split_two "$value"; then
                    check_absent "$CURRENT" "$SPLIT_L" "$SPLIT_R"
                else
                    fail "$CURRENT" "absent needs '<path> :: <regex>': $value"
                fi
                ;;
            test)
                if split_two "$value"; then
                    COUNT_TEST[$CURRENT]=$(( COUNT_TEST[$CURRENT] + 1 ))
                    check_test "$CURRENT" "$SPLIT_L" "$SPLIT_R" "behavioural test"
                else
                    fail "$CURRENT" "test needs '<path> :: <fn>': $value"
                fi
                ;;
            source_of_truth)
                if split_two "$value"; then
                    COUNT_SOT[$CURRENT]=$(( COUNT_SOT[$CURRENT] + 1 ))
                    check_test "$CURRENT" "$SPLIT_L" "$SPLIT_R" "source-of-truth test"
                else
                    fail "$CURRENT" "source_of_truth needs '<path> :: <fn>': $value"
                fi
                ;;
            order)
                if split_two "$value"; then
                    local rel="$SPLIT_L" rest="$SPLIT_R"
                    if split_two "$rest"; then
                        check_order "$CURRENT" "$rel" "$SPLIT_L" "$SPLIT_R"
                    else
                        fail "$CURRENT" "order needs '<path> :: <first> :: <second>': $value"
                    fi
                else
                    fail "$CURRENT" "order needs '<path> :: <first> :: <second>': $value"
                fi
                ;;
            *)
                fail "$CURRENT" "unknown directive '$key'"
                ;;
        esac
    done < "$MANIFEST"

    local finding
    for finding in $REQUIRED_FINDINGS; do
        if [[ " $seen_findings " != *" $finding "* ]]; then
            fail "$finding" "no evidence block in $MANIFEST; R0 cannot close without it"
            continue
        fi
    done

    for finding in $seen_findings; do
        case "${VERDICT[$finding]}" in
            closed) ;;
            open)   fail "$finding" "verdict is OPEN; R0 is blocked on this finding" ;;
            *)      fail "$finding" "verdict must be 'closed' or 'open', got '${VERDICT[$finding]}'" ;;
        esac

        case "${VERIFICATION[$finding]}" in
            mechanical) ;;
            manual)
                if [[ -z "${ATTESTOR[$finding]}" ]]; then
                    fail "$finding" "verification = manual requires a named attested_by"
                fi
                ;;
            *) fail "$finding" "verification must be 'mechanical' or 'manual', got '${VERIFICATION[$finding]}'" ;;
        esac

        (( COUNT_COMMIT[$finding] > 0 )) || fail "$finding" "no closure commit recorded"
        (( COUNT_ANCHOR[$finding] > 0 )) || fail "$finding" "no source anchor recorded"
        (( COUNT_WIRED[$finding]  > 0 )) || fail "$finding" "no live call site recorded (built-but-never-called is not closure)"
        (( COUNT_TEST[$finding]   > 0 )) || fail "$finding" "no behavioural test recorded"
        (( COUNT_SOT[$finding]    > 0 )) || fail "$finding" "no source-of-truth test recorded"
    done

    if (( COMMIT_SKIPS > 0 )); then
        echo "NOTE: $COMMIT_SKIPS commit ancestry check(s) skipped (git mode: $GIT_MODE)."
        echo "      CI runs this with fetch-depth: 0 and --require-commits, where skipping is a failure."
    fi

    return "$FAILED"
}

# --- self-test --------------------------------------------------------------

self_test() {
    # Deliberately not `local`: the EXIT trap below runs after this function
    # has returned, so a local would already be out of scope by then.
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT

    mkdir -p "$tmp/src"
    cat > "$tmp/src/subject.rs" <<'EOF'
fn keeper() {
    let candidates = indexed_lookup(db, query)?;
    sort_results(&mut results);
    results.truncate(limit);
}
#[test]
fn behaviour_is_preserved() {}
#[test]
fn persisted_rows_read_back() {}
EOF

    local head_sha
    head_sha="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "0000000000000000000000000000000000000000")"

    write_manifest() {
        cat > "$tmp/manifest.evidence" <<EOF
[finding:SELFTEST]
title = synthetic finding
verdict = $1
verification = mechanical
commit = $head_sha  synthetic
anchor = src/subject.rs :: $2
absent = src/subject.rs :: $3
order = src/subject.rs :: sort_results\(&mut results\); :: results\.truncate\(limit\);
wired = src/subject.rs :: indexed_lookup\(db, query\)
test = src/subject.rs :: $4
source_of_truth = src/subject.rs :: persisted_rows_read_back
EOF
    }

    local rc pass=0 fail_count=0
    expect() {
        local want="$1" label="$2"
        FAILED=0
        CURRENT=""
        MANIFEST="$tmp/manifest.evidence"
        BASE_DIR="$tmp"
        REQUIRED_FINDINGS="SELFTEST"
        run_checks >"$tmp/out.txt" 2>&1
        rc=$?
        if [[ "$rc" -eq "$want" ]]; then
            echo "  ok   $label (exit $rc)"
            pass=$((pass + 1))
        else
            echo "  BAD  $label — expected exit $want, got $rc"
            sed 's/^/         /' "$tmp/out.txt"
            fail_count=$((fail_count + 1))
        fi
    }

    echo "SELF-TEST: proving the R0 gate can go red"

    write_manifest "closed" "fn keeper\(\)" "read_all_rows" "behaviour_is_preserved"
    expect 0 "well-formed evidence passes"

    write_manifest "closed" "fn no_such_function\(\)" "read_all_rows" "behaviour_is_preserved"
    expect 1 "missing source anchor fails"

    write_manifest "closed" "fn keeper\(\)" "results\.truncate" "behaviour_is_preserved"
    expect 1 "reintroduced defect signature fails"

    write_manifest "closed" "fn keeper\(\)" "read_all_rows" "test_that_was_deleted"
    expect 1 "deleted behavioural test fails"

    write_manifest "open" "fn keeper\(\)" "read_all_rows" "behaviour_is_preserved"
    expect 1 "verdict 'open' fails"

    # A required finding with no block at all must fail.
    FAILED=0; CURRENT=""
    MANIFEST="$tmp/manifest.evidence"; BASE_DIR="$tmp"; REQUIRED_FINDINGS="SELFTEST MISSING42"
    write_manifest "closed" "fn keeper\(\)" "read_all_rows" "behaviour_is_preserved"
    if run_checks >"$tmp/out.txt" 2>&1; then
        echo "  BAD  missing required finding did not fail"
        fail_count=$((fail_count + 1))
    else
        echo "  ok   missing required finding fails (exit 1)"
        pass=$((pass + 1))
    fi

    # Order inversion: swap sort and truncate in the subject file.
    cat > "$tmp/src/subject.rs" <<'EOF'
fn keeper() {
    let candidates = indexed_lookup(db, query)?;
    results.truncate(limit);
    sort_results(&mut results);
}
#[test]
fn behaviour_is_preserved() {}
#[test]
fn persisted_rows_read_back() {}
EOF
    write_manifest "closed" "fn keeper\(\)" "read_all_rows" "behaviour_is_preserved"
    REQUIRED_FINDINGS="SELFTEST"
    expect 1 "sort-after-truncate order inversion fails"

    echo "SELF-TEST: $pass passed, $fail_count failed"
    [[ "$fail_count" -eq 0 ]]
}

if (( SELF_TEST )); then
    if self_test; then
        echo "SELF-TEST PASS"
        exit 0
    fi
    echo "SELF-TEST FAIL: the R0 gate did not go red where it must"
    exit 1
fi

echo "R0 entry gate — re-verifying ${MANIFEST#"$REPO_ROOT/"}"
if run_checks; then
    echo ""
    echo "R0 ENTRY GATE: PASS — findings 9, 11, 17, 40-43 and slice-1 shadow"
    echo "containment re-verified against source, tests and git history."
    echo "Behaviour-changing roadmap slices may be promoted subject to their own"
    echo "quantitative gates (learning-roadmap-r1-r8-w5-w6.md line 294)."
    exit 0
fi

echo ""
echo "R0 ENTRY GATE: FAIL — a learning prerequisite regressed or is unproven."
echo "No behaviour-changing roadmap slice (R1-R8, W5-W6) may be promoted while"
echo "this is red. Reference: docs/development/learning-roadmap-r1-r8-w5-w6.md:35"
exit 1
