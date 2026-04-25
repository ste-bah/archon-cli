#!/usr/bin/env bash
# install-hooks.sh — TASK-AGS-627
#
# Installs a git pre-commit hook that runs scripts/check-file-sizes.sh
# (FileSizeGuard for NFR-FOR-D4-MAINTAINABILITY) on every commit.
#
# The hook is installed into the GIT COMMON DIR (not the per-worktree dir),
# so it fires on commits made from any linked worktree as well as the main
# checkout.
#
# Usage:
#   bash scripts/install-hooks.sh         # safe install; refuses overwrite
#   bash scripts/install-hooks.sh --force # backup existing hook then overwrite
#
# Exit: 0 on success, non-zero on error (with message on stderr).

set -euo pipefail

# Resolve the repo root from the script's own location, NOT cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

FORCE=0
if [ "${1-}" = "--force" ]; then
  FORCE=1
fi

err() {
  printf 'install-hooks: %s\n' "$1" >&2
}

# Resolve the git common dir (shared across worktrees). We deliberately use
# --git-common-dir, not --git-dir, so the hook installs to the canonical
# .git/hooks even when invoked from a linked worktree.
GIT_COMMON_DIR="$( cd "$REPO_ROOT" && git rev-parse --git-common-dir 2>/dev/null || true )"
if [ -z "$GIT_COMMON_DIR" ]; then
  err "could not resolve git common dir; is $REPO_ROOT inside a git repo?"
  exit 2
fi

# git rev-parse may return a relative path; resolve against repo root.
case "$GIT_COMMON_DIR" in
  /*) ABS_GIT_COMMON_DIR="$GIT_COMMON_DIR" ;;
  *)  ABS_GIT_COMMON_DIR="$( cd "$REPO_ROOT/$GIT_COMMON_DIR" 2>/dev/null && pwd || true )" ;;
esac
if [ -z "$ABS_GIT_COMMON_DIR" ] || [ ! -d "$ABS_GIT_COMMON_DIR" ]; then
  err "git common dir does not exist: $GIT_COMMON_DIR"
  exit 2
fi

HOOKS_DIR="$ABS_GIT_COMMON_DIR/hooks"
HOOK_PATH="$HOOKS_DIR/pre-commit"

mkdir -p "$HOOKS_DIR"

# The hook content we want to install. Note we resolve the repo root at
# runtime via `git rev-parse --show-toplevel`, so the hook works no matter
# which worktree the commit is made from.
read -r -d '' DESIRED_HOOK <<'HOOK_EOF' || true
#!/usr/bin/env bash
# Installed by scripts/install-hooks.sh (TASK-AGS-627)
exec bash "$(git rev-parse --show-toplevel)/scripts/check-file-sizes.sh"
HOOK_EOF

if [ -e "$HOOK_PATH" ]; then
  # Compare existing content to desired.
  EXISTING_CONTENT="$(cat "$HOOK_PATH")"
  if [ "$EXISTING_CONTENT" = "$DESIRED_HOOK" ]; then
    printf 'pre-commit hook already up to date at %s\n' "$HOOK_PATH"
    exit 0
  fi
  if [ "$FORCE" -ne 1 ]; then
    err "a pre-commit hook already exists at $HOOK_PATH and differs from the one this installer would write."
    err "existing hook contents:"
    while IFS= read -r line; do
      printf 'install-hooks:   | %s\n' "$line" >&2
    done <<<"$EXISTING_CONTENT"
    err "refusing to overwrite. Re-run with --force to backup and replace."
    exit 3
  fi
  # --force: backup then overwrite.
  TS="$(date +%Y%m%d%H%M%S)"
  BACKUP="$HOOK_PATH.bak.$TS"
  cp "$HOOK_PATH" "$BACKUP"
  printf 'install-hooks: backed up existing hook to %s\n' "$BACKUP"
fi

printf '%s\n' "$DESIRED_HOOK" > "$HOOK_PATH"
chmod +x "$HOOK_PATH"

# Sanity-check the install.
if [ ! -s "$HOOK_PATH" ]; then
  err "post-install verification failed: $HOOK_PATH is empty"
  exit 4
fi
if ! head -c 2 "$HOOK_PATH" | grep -q '#!'; then
  err "post-install verification failed: $HOOK_PATH does not begin with shebang"
  exit 4
fi

printf 'Installed pre-commit hook at %s\n' "$HOOK_PATH"
exit 0
