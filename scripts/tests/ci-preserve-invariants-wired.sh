#!/usr/bin/env bash
# Gate-1 test for TASK-AGS-OBS-918 CI-wire.
# Asserts ci.yml (or tui-observability.yml) has a job that invokes
# scripts/check-preserve-invariants.sh and that workflow_dispatch trigger is set.
#
# Exits 0 on GREEN (wiring present), non-zero on RED.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
TUI_YML="$REPO_ROOT/.github/workflows/tui-observability.yml"

if [[ ! -f "$CI_YML" ]]; then
    echo "FAIL: $CI_YML not found"
    exit 1
fi

python3 - "$CI_YML" "$TUI_YML" <<'PY'
import sys, yaml, pathlib

ci_path, tui_path = sys.argv[1], sys.argv[2]
found_job = False
found_dispatch = False
offending = []

for p in (ci_path, tui_path):
    if not pathlib.Path(p).is_file():
        continue
    with open(p, "r", encoding="utf-8") as f:
        try:
            doc = yaml.safe_load(f)
        except yaml.YAMLError as e:
            print(f"FAIL: YAML parse error in {p}: {e}")
            sys.exit(1)
    if not isinstance(doc, dict):
        continue

    # Detect workflow_dispatch trigger.
    on = doc.get(True) or doc.get("on")  # PyYAML parses bare 'on:' key as boolean True
    if isinstance(on, dict) and "workflow_dispatch" in on:
        found_dispatch = True
    elif isinstance(on, list) and "workflow_dispatch" in on:
        found_dispatch = True
    elif on == "workflow_dispatch":
        found_dispatch = True

    # Detect job that invokes scripts/check-preserve-invariants.sh.
    jobs = doc.get("jobs") or {}
    for job_name, job in jobs.items():
        if not isinstance(job, dict):
            continue
        for step in job.get("steps") or []:
            if not isinstance(step, dict):
                continue
            run = step.get("run", "")
            if "check-preserve-invariants.sh" in run:
                found_job = True
                print(f"OK: job={job_name} in {pathlib.Path(p).name} invokes check-preserve-invariants.sh")
                break
        else:
            continue
        break

if not found_job:
    offending.append("no job invokes scripts/check-preserve-invariants.sh in ci.yml or tui-observability.yml")
if not found_dispatch:
    offending.append("no workflow_dispatch trigger found in ci.yml or tui-observability.yml")

if offending:
    print("RED:")
    for o in offending:
        print(f"  - {o}")
    sys.exit(1)

print("GREEN: preserve-invariants wired to CI with workflow_dispatch trigger")
PY
