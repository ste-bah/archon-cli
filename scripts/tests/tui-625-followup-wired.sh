#!/usr/bin/env bash
# Gate-1 structural check for TASK-TUI-625-FOLLOWUP.
# Verifies the real-startup wiring is landed:
#   1. src/cli_args.rs declares remote_url: Option<String> with long="remote-url"
#   2. src/main.rs sets ARCHON_REMOTE_URL from cli.remote_url
#   3. src/command/session.rs no longer carries "TUI-625-followup ticket" deferral marker
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

FAIL=0

chk() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "OK: $name"
    else
        echo "RED: $name — pattern '$pattern' not found in $file"
        FAIL=1
    fi
}

chk_no() {
    local name=$1 file=$2 pattern=$3
    if grep -qE "$pattern" "$REPO_ROOT/$file" 2>/dev/null; then
        echo "RED: $name — '$pattern' still present in $file"
        FAIL=1
    else
        echo "OK: $name"
    fi
}

chk "cli_args.rs declares remote_url field" \
    "src/cli_args.rs" \
    "pub remote_url: Option<String>"

chk "cli_args.rs gives remote_url long=\"remote-url\"" \
    "src/cli_args.rs" \
    "long = \"remote-url\""

chk "main.rs sets ARCHON_REMOTE_URL from cli.remote_url" \
    "src/main.rs" \
    "set_var.*ARCHON_REMOTE_URL"

chk_no "session.rs retains no TUI-625-followup deferral marker" \
    "src/command/session.rs" \
    "TUI-625-followup ticket"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: TUI-625-followup real-startup wiring present"
