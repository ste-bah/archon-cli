#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.1a (#178) Multi-modal image input.
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
    echo "OK: multimodal.rs present"
elif [[ -f "$REPO_ROOT/crates/archon-llm/src/multimodal/mod.rs" ]]; then
    MM_FILE="crates/archon-llm/src/multimodal/mod.rs"
    echo "OK: multimodal/mod.rs present"
else
    echo "RED: neither crates/archon-llm/src/multimodal.rs nor multimodal/mod.rs exists"
    FAIL=1
fi

chk "ContentBlock has Image variant" \
    "crates/archon-llm/src/types.rs" \
    "Image \\{|ContentBlock::Image"

if [[ -n "$MM_FILE" ]]; then
    chk "ImageSource struct / enum" \
        "$MM_FILE" \
        "pub struct ImageSource|pub enum ImageSource"

    chk "image_block_from_bytes helper" \
        "$MM_FILE" \
        "pub fn image_block_from_bytes"

    chk "PNG magic validation" \
        "$MM_FILE" \
        "0x89.*0x50.*0x4E.*0x47|PNG magic|png_signature|89 50 4E 47"
fi

chk "archon-llm lib.rs declares pub mod multimodal" \
    "crates/archon-llm/src/lib.rs" \
    "pub mod multimodal"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.1a multi-modal image input surfaces present"
