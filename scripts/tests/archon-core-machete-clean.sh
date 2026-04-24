#!/usr/bin/env bash
# Gate-1 test for TASK-AGS-CARGO-MACHETE-CORE.
# Asserts cargo-machete reports archon-core has zero unused deps AND that the
# 5 deps identified as unused (archon-mcp, clap, tower-http, tracing-appender,
# tracing-subscriber) are no longer present in archon-core/Cargo.toml
# [dependencies].
#
# Exits 0 GREEN, non-zero RED.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO_TOML="$REPO_ROOT/crates/archon-core/Cargo.toml"
MACHETE="${CARGO_MACHETE_BIN:-/home/unixdude/.cargo/bin/cargo-machete}"

if [[ ! -f "$CARGO_TOML" ]]; then
    echo "RED: $CARGO_TOML not found"
    exit 1
fi
if [[ ! -x "$MACHETE" ]]; then
    echo "RED: cargo-machete not found at $MACHETE (set CARGO_MACHETE_BIN)"
    exit 1
fi

FAIL=0

# Extract just the [dependencies] section so we don't accidentally match
# package name, dev-dependencies, or features.
DEPS_SECTION=$(awk '/^\[dependencies\]/{flag=1;next} /^\[/{flag=0} flag' "$CARGO_TOML")

for dep in tracing-appender tracing-subscriber archon-mcp clap tower-http; do
    # Look for line starting with the dep name followed by . or space or =
    # (tolerant to workspace re-export syntax like `clap.workspace = true`).
    if echo "$DEPS_SECTION" | grep -E "^${dep}([.[:space:]]|=)" >/dev/null; then
        echo "RED: ${dep} still present in [dependencies] of archon-core"
        FAIL=1
    else
        echo "OK: ${dep} absent from [dependencies]"
    fi
done

# Run machete against archon-core — must report zero unused.
pushd "$REPO_ROOT/crates/archon-core" >/dev/null
MACHETE_OUT=$("$MACHETE" . 2>&1) || true
popd >/dev/null

if echo "$MACHETE_OUT" | grep -qE "archon-core -- "; then
    echo "RED: cargo-machete still flags unused deps in archon-core:"
    echo "$MACHETE_OUT"
    FAIL=1
else
    echo "OK: cargo-machete reports zero unused deps in archon-core"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "FAIL"
    exit 1
fi

echo ""
echo "GREEN: archon-core has no unused deps (5 removed, cargo-machete clean)"
