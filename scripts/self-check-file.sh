#!/usr/bin/env bash
# self-check-file.sh — per-edit FileSizeGuard for the file the agent just wrote.
#
# Wired to PostToolUse in .archon/hooks.toml. Reads the hook payload as JSON on
# stdin, pulls out the edited file, and applies the SAME rule as
# scripts/check-file-sizes.sh — 500 lines, exact whole-line allowlist match — to
# that one file.
#
# WHY this exists: PostToolUse is the only hook event whose output reaches the
# model. Stdout on exit 0 becomes `additional_context`, which is appended to the
# tool result as "[Hook Context]" and enters the transcript, so the agent reads
# a gate violation it just created on its next turn instead of hearing about it
# from CI or from the user three commits later.
#
# WHY not just call check-file-sizes.sh: that walks ~3160 files and spawns one
# `wc` per file — measured at roughly 165 s on a Windows dev box. Once per edit
# that is unusable. This reuses its *rule*, not its walk: measured at ~0.4 s per
# edit on the same box, nearly all of it shell startup.
#
# WHY silence is a pass: everything printed here is spent transcript budget on
# every single edit. The script speaks only when the invariant is actually
# broken.
#
# Usage: hook only. $1 is the triggering tool name, used in the verdict text.
# Exit:  always 0. A self-check must never block a turn.

set -u

# Keep in sync with THRESHOLD in scripts/check-file-sizes.sh:14.
THRESHOLD=500

TOOL="${1:-edit}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ALLOWLIST_FILE="$REPO_ROOT/scripts/check-file-sizes.allowlist"

payload="$(cat)"

# Extract the first `"file_path":"..."` from the compact JSON payload without a
# JSON parser — jq is not guaranteed on a developer box. A mis-extraction can
# only yield a path that fails one of the guards below, so the worst case is
# silence, never a false accusation. File paths containing a literal `"` are not
# handled and are not worth handling.
case "$payload" in
  *'"file_path":"'*) ;;
  *) exit 0 ;;  # non-file tool (or file_path null) — nothing to check
esac
rest="${payload#*'"file_path":"'}"
raw="${rest%%\"*}"
[ -n "$raw" ] || exit 0

# JSON-unescape backslashes, then normalise Windows separators so the path is
# usable from the POSIX shell the hook runs under. Done with parameter expansion
# rather than sed|tr because this runs on every edit and each spawn is ~50 ms on
# Windows.
bs='\'
file_path="${raw//"$bs$bs"/"$bs"}"
file_path="${file_path//"$bs"//}"

# Same extension set the full gate walks; complaining about a long Markdown file
# would be crying wolf about a rule that does not exist. Checked before any
# filesystem work so the common non-source edit costs nothing.
case "$file_path" in
  *.rs|*.ts|*.tsx|*.css) ;;
  *) exit 0 ;;
esac

# Round-trip through the shell's own cwd so the payload path and REPO_ROOT are
# expressed in the same syntax (Git Bash reports `F:/x` as `/f/x`). A path that
# cannot be entered — deleted directory, bad drive — exits quietly.
file_dir="$(cd "$(dirname "$file_path")" 2>/dev/null && pwd)" || exit 0
[ -n "$file_dir" ] || exit 0
abs="$file_dir/$(basename "$file_path")"

[ -f "$abs" ] || exit 0  # file deleted or moved since the tool ran

# Walk up from the file to the repo root comparing device+inode rather than
# strings: Git Bash reports the same directory as `/c/Users/.../Temp/x` or
# `/tmp/x` depending on which spelling it was reached by, so a prefix match on
# text silently drops real edits. `-ef` is immune to that, and building `rel`
# with parameter expansion keeps the walk free of subprocess spawns.
dir="$file_dir"
rel="${file_path##*/}"
inside=0
while [ -n "$dir" ]; do
  if [ "$dir" -ef "$REPO_ROOT" ]; then
    inside=1
    break
  fi
  [ "$dir" = "/" ] && break
  rel="${dir##*/}/$rel"
  parent="${dir%/*}"
  [ "$parent" = "$dir" ] && break
  dir="${parent:-/}"
done
[ "$inside" -eq 1 ] || exit 0  # outside the repo — the gate has no opinion

lines="$(wc -l < "$abs")"
lines="${lines// /}"  # some `wc` builds pad the count
[ "$lines" -gt "$THRESHOLD" ] || exit 0

# Allowlist entries are exact repo-relative paths, matched whole-line after
# comment stripping and trimming — identical to check-file-sizes.sh:26,44.
if [ -f "$ALLOWLIST_FILE" ] \
  && sed 's/#.*//' "$ALLOWLIST_FILE" | awk '{$1=$1;print}' | grep -Fxq -- "$rel"; then
  exit 0  # grandfathered debt — CI tolerates it, so this must not nag
fi

printf 'FileSizeGuard: %s is now %d lines after that %s — over the %d-line limit and not in scripts/check-file-sizes.allowlist. CI will fail. Split it before moving on.\n' \
  "$rel" "$lines" "$TOOL" "$THRESHOLD"
exit 0
