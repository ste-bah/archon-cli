#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.1b (#179) Multi-modal PDF input.
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

MM_FILE=""
if [[ -f "$REPO_ROOT/crates/archon-llm/src/multimodal.rs" ]]; then
    MM_FILE="crates/archon-llm/src/multimodal.rs"
elif [[ -f "$REPO_ROOT/crates/archon-llm/src/multimodal/mod.rs" ]]; then
    MM_FILE="crates/archon-llm/src/multimodal/mod.rs"
else
    echo "RED: multimodal module missing"
    FAIL=1
fi

chk "ContentBlock has Document variant" \
    "crates/archon-llm/src/types.rs" \
    "Document \\{|ContentBlock::Document"

if [[ -n "$MM_FILE" ]]; then
    chk "DocumentSource struct" \
        "$MM_FILE" \
        "pub struct DocumentSource|pub enum DocumentSource"

    chk "document_block_from_bytes helper" \
        "$MM_FILE" \
        "pub fn document_block_from_bytes"

    chk "PDF magic validation" \
        "$MM_FILE" \
        "0x25.*0x50.*0x44.*0x46|%PDF|PDF_MAGIC"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.1b multi-modal PDF input surfaces present"
