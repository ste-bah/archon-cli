#!/usr/bin/env bash
# check-complexity.sh — enforce AC-OBSERVABILITY-02 cognitive-complexity gate.
#
# Denies clippy::cognitive_complexity on the archon-tui crate. Threshold is
# defined in the workspace-root clippy.toml (cognitive-complexity-threshold = 10).
# Any function with cyclomatic complexity >= 10 will fail the build.
set -euo pipefail

cargo clippy -p archon-tui --all-targets -- -D clippy::cognitive_complexity 2>&1

echo "OK: clippy cognitive_complexity clean"
