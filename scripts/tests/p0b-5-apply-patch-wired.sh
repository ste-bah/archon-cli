#!/usr/bin/env bash
# Gate-1 structural check for TASK-P0-B.5 (#183) apply_patch tool.
# Verifies:
#   1. crates/archon-tools/src/apply_patch/mod.rs exists (post-split layout)
#      — originally a single apply_patch.rs file; split into submodules
#      (parser.rs, applier.rs, mod.rs) to satisfy the 500-line
#      FileSizeGuard invariant. Public API (archon_tools::apply_patch::ApplyPatchTool)
#      is unchanged.
#   2. ApplyPatchTool struct + impl Tool
#   3. archon-tools lib.rs declares pub mod apply_patch
#   4. dispatch.rs registers ApplyPatchTool
#   5. input_schema declares required path + patch
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

# Post-split layout: the tool lives in apply_patch/mod.rs. The older
# single-file layout (apply_patch.rs) is also accepted so this check
# keeps working on historical branches.
APPLY_PATCH_FILE=""
if [[ -f "$REPO_ROOT/crates/archon-tools/src/apply_patch/mod.rs" ]]; then
    APPLY_PATCH_FILE="crates/archon-tools/src/apply_patch/mod.rs"
    echo "OK: apply_patch/mod.rs present (post-split layout)"
elif [[ -f "$REPO_ROOT/crates/archon-tools/src/apply_patch.rs" ]]; then
    APPLY_PATCH_FILE="crates/archon-tools/src/apply_patch.rs"
    echo "OK: apply_patch.rs present (legacy single-file layout)"
else
    echo "RED: neither crates/archon-tools/src/apply_patch/mod.rs nor apply_patch.rs exists"
    FAIL=1
fi

if [[ -n "$APPLY_PATCH_FILE" ]]; then
    chk "ApplyPatchTool struct" \
        "$APPLY_PATCH_FILE" \
        "pub struct ApplyPatchTool"

    chk "impl Tool for ApplyPatchTool" \
        "$APPLY_PATCH_FILE" \
        "impl Tool for ApplyPatchTool"

    chk "input_schema declares required path + patch" \
        "$APPLY_PATCH_FILE" \
        "\"required\"|required.*\\[.*path.*patch|required.*\\[.*patch.*path"
fi

chk "archon-tools lib.rs declares pub mod apply_patch" \
    "crates/archon-tools/src/lib.rs" \
    "pub mod apply_patch"

chk "dispatch.rs registers ApplyPatchTool" \
    "crates/archon-core/src/dispatch.rs" \
    "apply_patch::ApplyPatchTool"

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "check failed"
    exit 1
fi
echo ""
echo "GREEN: P0-B.5 apply_patch tool surfaces present"
