#!/usr/bin/env bash
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

MM="crates/archon-llm/src/multimodal.rs"

chk "ContentBlock has Audio variant" \
    "crates/archon-llm/src/types.rs" \
    "Audio \\{|ContentBlock::Audio"

chk "AudioSource struct" \
    "$MM" \
    "pub struct AudioSource|pub enum AudioSource"

chk "audio_block_from_bytes helper" \
    "$MM" \
    "pub fn audio_block_from_bytes"

chk "WAV magic validation (RIFF + WAVE)" \
    "$MM" \
    "WAV_RIFF|WAV_TAG|audio/wav"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.1c multi-modal audio input surfaces present"
