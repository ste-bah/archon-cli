#!/usr/bin/env bash
# check-complexity.sh — enforce AC-OBSERVABILITY-02 cognitive-complexity gate.
#
# Checks clippy::cognitive_complexity against the archon-tui crate. Threshold is
# defined in the workspace-root clippy.toml (cognitive-complexity-threshold = 60).
# NOTE: Scoped to archon-tui only via --no-deps; workspace-wide legacy debt
# is tracked separately. Uses -D (deny) so any violation in archon-tui blocks.
set -euo pipefail

cargo clippy -p archon-tui --no-deps --all-targets -- -D clippy::cognitive_complexity 2>&1

echo "OK: clippy cognitive_complexity clean"
