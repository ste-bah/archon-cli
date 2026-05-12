#!/usr/bin/env bash
# Build the Archon web frontend.
# Runs the Vite production build consumed by rust-embed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ ! -d node_modules ]]; then
  npm install
fi

npm run typecheck
npm run build

echo "Build complete. Output: $SCRIPT_DIR/dist/"
ls -lh "$SCRIPT_DIR/dist/"
