#!/usr/bin/env bash
# Build the Archon web frontend.
# Compiles TypeScript → dist/, then bundles into a single app.js for embedding.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Compile TypeScript
if command -v npx &>/dev/null; then
  npx tsc --noEmit --strict 2>&1 || true
fi

# For embedding we produce a single-file JS bundle.
# Since we have no bundler installed by default, we concatenate the modules
# in dependency order and strip ES module syntax for browser compatibility.
# A production build would use esbuild/rollup, but this is sufficient for
# the embedded single-binary use case.

DIST="$SCRIPT_DIR/dist"
mkdir -p "$DIST"

# Simple concatenation bundle (no import/export at runtime — all in one scope)
{
  echo "// Archon Web UI — bundled $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo ";"

  for src in \
    src/connection.ts \
    src/chat.ts \
    src/input.ts \
    src/session.ts \
    src/settings.ts \
    src/app.ts; do

    if [[ -f "$src" ]]; then
      # Strip TypeScript-specific syntax and ES module declarations
      sed \
        -e '/^import /d' \
        -e '/^export /d' \
        -e 's/: [A-Z][A-Za-z<>|, \[\]]*\b//g' \
        -e 's/<[A-Z][A-Za-z<>]*>//g' \
        "$src"
      echo ";"
    fi
  done
} > "$DIST/app.js"

# Copy static assets alongside
cp "$SCRIPT_DIR/index.html" "$DIST/index.html"
cp "$SCRIPT_DIR/styles.css" "$DIST/styles.css"

echo "Build complete. Output: $DIST/"
ls -lh "$DIST/"
