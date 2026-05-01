#!/bin/sh
# Install archon-cli's tracked git hooks via core.hooksPath.
#
# Run once after cloning the repo:
#     scripts/install-git-hooks.sh
#
# Subsequent pulls update the hooks automatically (they're tracked files,
# not in .git/hooks).

set -eu

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

HOOKS_DIR="scripts/git-hooks"

if [ ! -d "$HOOKS_DIR" ]; then
    echo "install-git-hooks: $HOOKS_DIR not found — run from repo root." >&2
    exit 1
fi

# Make every hook executable (idempotent).
chmod +x "$HOOKS_DIR"/* 2>/dev/null || true

# Point git at the tracked hooks dir. Idempotent.
git config core.hooksPath "$HOOKS_DIR"

echo "Installed git hooks via core.hooksPath = $HOOKS_DIR"
echo ""
echo "Active hooks:"
for hook in "$HOOKS_DIR"/*; do
    [ -f "$hook" ] || continue
    name="$(basename "$hook")"
    case "$name" in
        *.md|*.txt|README*) continue;;
    esac
    echo "  - $name"
done
echo ""
echo "Bypass any hook with: git <command> --no-verify"
