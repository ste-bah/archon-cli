#!/usr/bin/env bash
# r0-entry-gate-wired.sh
#
# Gate-1 wiring test for the R0 entry gate.
#
# This repository has repeatedly shipped subsystems that were built correctly
# and then never called (#76, #129, #161), and twice reported success from code
# that did nothing (#153, #162). An entry gate is the worst possible place for
# that: an uninvoked R0 checker would let every downstream roadmap slice claim
# promotion eligibility from a green board.
#
# So this asserts the invocation, not the implementation:
#   1. scripts/check-r0-entry-gate.sh exists and is executable as a bash script;
#   2. the evidence manifest it reads exists;
#   3. a CI job actually runs it, with --require-commits and fetch-depth: 0;
#   4. a CI step runs its --self-test;
#   5. scripts/ci-gate.sh runs it too, so a local pre-push gate catches it.
#
# Exits 0 on GREEN (wiring present), 1 on RED.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

CHECKER="scripts/check-r0-entry-gate.sh"
MANIFEST="docs/development/r0-entry-gate.evidence"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
CI_GATE="$REPO_ROOT/scripts/ci-gate.sh"

FAIL=0

red() {
    echo "RED: $1"
    FAIL=1
}

for required in "$CHECKER" "$MANIFEST"; do
    if [[ -f "$REPO_ROOT/$required" ]]; then
        echo "OK: $required present"
    else
        red "$required missing"
    fi
done

if [[ ! -f "$CI_YML" ]]; then
    red "$CI_YML missing"
else
    python3 - "$CI_YML" "$CHECKER" <<'PY'
import pathlib
import sys

import yaml

ci_path, checker = sys.argv[1], sys.argv[2]
doc = yaml.safe_load(pathlib.Path(ci_path).read_text(encoding="utf-8"))
if not isinstance(doc, dict):
    print(f"RED: {ci_path} did not parse as a mapping")
    sys.exit(1)

problems = []
runs_checker = None
runs_self_test = False

for job_name, job in (doc.get("jobs") or {}).items():
    if not isinstance(job, dict):
        continue
    steps = job.get("steps") or []
    invocations = [
        s.get("run", "")
        for s in steps
        if isinstance(s, dict) and checker in s.get("run", "")
    ]
    if not invocations:
        continue
    runs_checker = job_name
    print(f"OK: job={job_name} invokes {checker}")

    if not any("--require-commits" in run for run in invocations):
        problems.append(
            f"job={job_name} runs the checker without --require-commits; commit"
            " ancestry would be skipped silently in CI"
        )
    if any("--self-test" in run for run in invocations):
        runs_self_test = True

    # fetch-depth: 0 is what makes --require-commits satisfiable.
    depths = [
        (s.get("with") or {}).get("fetch-depth")
        for s in steps
        if isinstance(s, dict) and str(s.get("uses", "")).startswith("actions/checkout")
    ]
    if 0 not in [d if isinstance(d, int) else None for d in depths]:
        problems.append(
            f"job={job_name} does not check out with fetch-depth: 0, so"
            " --require-commits cannot pass"
        )
    break

if runs_checker is None:
    problems.append(f"no job in {pathlib.Path(ci_path).name} invokes {checker}")
elif not runs_self_test:
    problems.append(
        f"job={runs_checker} never runs the checker's --self-test; an entry gate"
        " nobody has watched go red is decoration"
    )

if problems:
    for problem in problems:
        print(f"RED: {problem}")
    sys.exit(1)

print("OK: CI job wiring complete (checker + --require-commits + fetch-depth 0 + self-test)")
PY
    if [[ $? -ne 0 ]]; then
        FAIL=1
    fi
fi

if [[ ! -f "$CI_GATE" ]]; then
    red "$CI_GATE missing"
elif grep -q "$CHECKER" "$CI_GATE"; then
    echo "OK: scripts/ci-gate.sh invokes $CHECKER"
else
    red "scripts/ci-gate.sh does not invoke $CHECKER"
fi

if [[ "$FAIL" -ne 0 ]]; then
    echo ""
    echo "R0 entry gate is not wired. Fix the invocation before relying on it."
    exit 1
fi

echo ""
echo "GREEN: R0 entry gate is invoked by CI and by scripts/ci-gate.sh"
