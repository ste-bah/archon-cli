#!/usr/bin/env bash
# Self-test: verify .github/workflows/tui-observability.yml is valid YAML and,
# if actionlint is installed, that it passes actionlint.
# Spec: TASK-TUI-814 Validation Criteria 1 + 6.
set -euo pipefail

WORKFLOW="${WORKFLOW_FILE:-.github/workflows/tui-observability.yml}"

if [[ ! -f "$WORKFLOW" ]]; then
    echo "FAIL: workflow not found at $WORKFLOW" >&2
    exit 2
fi

echo "==> Checking YAML validity of $WORKFLOW"
python3 - "$WORKFLOW" <<'PY'
import sys
import yaml

path = sys.argv[1]
with open(path) as f:
    data = yaml.safe_load(f)

if not isinstance(data, dict):
    sys.stderr.write(f"FAIL: top-level YAML is not a mapping in {path}\n")
    sys.exit(1)

jobs = data.get("jobs")
if not isinstance(jobs, dict) or not jobs:
    sys.stderr.write("FAIL: 'jobs:' block missing or empty\n")
    sys.exit(1)

if len(jobs) != 9:
    sys.stderr.write(f"FAIL: expected 9 jobs, found {len(jobs)}: {sorted(jobs)}\n")
    sys.exit(1)

required_ids = {
    "tui-lint-filesize",
    "tui-lint-complexity",
    "tui-lint-cycles",
    "tui-lint-duplication",
    "tui-lint-bounded-channel",
    "tui-lint-await-send",
    "tui-coverage",
    "tui-load-tests",
    "tui-unit",
}
missing = required_ids - set(jobs)
if missing:
    sys.stderr.write(f"FAIL: missing expected job ids: {sorted(missing)}\n")
    sys.exit(1)

# Count allowed-to-fail jobs; spec requires exactly 6.
soft = sum(1 for j in jobs.values() if j.get("continue-on-error") is True)
if soft != 6:
    sys.stderr.write(
        f"FAIL: expected 6 jobs with continue-on-error: true, found {soft}\n"
    )
    sys.exit(1)

print(f"YAML OK: {len(jobs)} jobs, {soft} allowed-to-fail, 3 required")
PY

echo "==> Running actionlint (if available)"
if command -v actionlint >/dev/null 2>&1; then
    actionlint "$WORKFLOW"
    echo "actionlint: OK"
else
    echo "actionlint: not installed, skipping"
fi

echo "PASS: $WORKFLOW"
exit 0
