#!/usr/bin/env bash
# tui-banned-patterns-gate.sh
#
# Grep-based banned-patterns CI gate with a ratchet allow-list.
#
# Environment:
#   GATE_ROOT            - repo root (default: git rev-parse --show-toplevel, fallback pwd)
#   BANNED_PATTERNS_JSON - path to config (default: $GATE_ROOT/config/tui-banned-patterns.json)
#   SCAN_ROOTS           - space-separated dirs (default: "src crates/archon-tui/src")
#
# Exit codes:
#   0  OK (no unknown violations)
#   1  One or more unknown (non-allow-listed) violations
#   2  Preflight failure (missing jq, missing/malformed config)

set -euo pipefail
shopt -s extglob globstar nullglob

# ---- Preflight -------------------------------------------------------------

if ! command -v jq >/dev/null 2>&1; then
  echo "FATAL: jq is required but not found in PATH" >&2
  exit 2
fi

if [ -z "${GATE_ROOT:-}" ]; then
  if GR="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    GATE_ROOT="$GR"
  else
    GATE_ROOT="$PWD"
  fi
fi

BANNED_PATTERNS_JSON="${BANNED_PATTERNS_JSON:-$GATE_ROOT/config/tui-banned-patterns.json}"
SCAN_ROOTS="${SCAN_ROOTS:-src crates/archon-tui/src}"

if [ ! -r "$BANNED_PATTERNS_JSON" ]; then
  echo "FATAL: cannot read config: $BANNED_PATTERNS_JSON" >&2
  exit 2
fi

if ! jq -e . "$BANNED_PATTERNS_JSON" >/dev/null 2>&1; then
  echo "FATAL: malformed JSON in $BANNED_PATTERNS_JSON" >&2
  exit 2
fi

# ---- Load rules ------------------------------------------------------------

RULE_IDS=()
RULE_REGEXES=()
RULE_OWNER_PHASES=()
RULE_PATH_GLOBS=()

while IFS= read -r line; do
  RULE_IDS+=("$line")
done < <(jq -r '.rules[].id' "$BANNED_PATTERNS_JSON")

while IFS= read -r line; do
  RULE_REGEXES+=("$line")
done < <(jq -r '.rules[].regex' "$BANNED_PATTERNS_JSON")

while IFS= read -r line; do
  RULE_OWNER_PHASES+=("$line")
done < <(jq -r '.rules[].owner_phase' "$BANNED_PATTERNS_JSON")

while IFS= read -r line; do
  RULE_PATH_GLOBS+=("$line")
done < <(jq -r '.rules[].path_glob // ""' "$BANNED_PATTERNS_JSON")

NUM_RULES="${#RULE_IDS[@]}"

# ---- Load allowlist --------------------------------------------------------

declare -A ALLOWED=()

while IFS= read -r entry; do
  [ -z "$entry" ] && continue
  apath="${entry%%|*}"
  arule="${entry##*|}"
  ALLOWED["$apath|$arule"]=1
done < <(jq -r '.allowlist[] | "\(.path)|\(.rule)"' "$BANNED_PATTERNS_JSON")

# ---- Glob matching helper --------------------------------------------------

# Convert a JSON glob like "crates/archon-tui/**" to a bash extglob pattern.
# Since we use `shopt -s globstar`, `**` already matches across path segments
# inside `[[ $str == $glob ]]`, so we can pass globs through as-is.
match_glob() {
  local str="$1"
  local glob="$2"
  [ -z "$glob" ] && return 0
  # shellcheck disable=SC2053
  [[ $str == $glob ]]
}

# ---- Scan ------------------------------------------------------------------

VIOLATIONS=()
ALLOWLISTED_COUNT=0
GATE_ROOT_NOSLASH="${GATE_ROOT%/}"

for root in $SCAN_ROOTS; do
  abs_root="$GATE_ROOT_NOSLASH/$root"
  [ -d "$abs_root" ] || continue

  while IFS= read -r abs_file; do
    rel_path="${abs_file#${GATE_ROOT_NOSLASH}/}"

    for ((i = 0; i < NUM_RULES; i++)); do
      rule_id="${RULE_IDS[$i]}"
      regex="${RULE_REGEXES[$i]}"
      owner="${RULE_OWNER_PHASES[$i]}"
      glob="${RULE_PATH_GLOBS[$i]}"

      if [ -n "$glob" ]; then
        if ! match_glob "$rel_path" "$glob"; then
          continue
        fi
      fi

      # grep -nE; tolerate zero matches.
      hits="$(grep -nE -- "$regex" "$abs_file" 2>/dev/null || true)"
      [ -z "$hits" ] && continue

      allow_key="$rel_path|$rule_id"
      if [ "${ALLOWED[$allow_key]+x}" = "x" ]; then
        while IFS= read -r _hit; do
          ALLOWLISTED_COUNT=$((ALLOWLISTED_COUNT + 1))
        done <<< "$hits"
        continue
      fi

      while IFS= read -r hit; do
        line_num="${hit%%:*}"
        VIOLATIONS+=("FAIL: $rel_path:$line_num rule=$rule_id owner=$owner (not allow-listed)")
      done <<< "$hits"
    done
  done < <(find "$abs_root" -type f -name '*.rs')
done

# ---- Report ----------------------------------------------------------------

if [ "${#VIOLATIONS[@]}" -gt 0 ]; then
  for v in "${VIOLATIONS[@]}"; do
    echo "$v" >&2
  done
  echo "${#VIOLATIONS[@]} violations found" >&2
  exit 1
fi

echo "OK: $NUM_RULES rules checked, $ALLOWLISTED_COUNT matches allow-listed, 0 unknown violations"
exit 0
