#!/usr/bin/env bash
# gen-discovery-fixture.sh — Deterministic 300-agent discovery fixture
# generator (REQ-DISCOVERY-002/004, NFR-PERF-002 benchmark support).
#
# Writes tests/fixtures/agents/generated/<category>/agent_NNN.yaml for a
# fixed category distribution (total 300). Idempotent: wipes the
# generated/ subtree first. Deterministic: no timestamps, no RNG.
#
# Spec: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-004.md
# Usage: bash scripts/gen-discovery-fixture.sh
# Exit:  0 on success, non-zero on error.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

OUT_ROOT="tests/fixtures/agents/generated"

# Category: count pairs. Order is fixed so output is deterministic.
CATEGORIES=(
  "custom:11"
  "development:60"
  "coding-pipeline:50"
  "core:30"
  "analysis:30"
  "hive-mind:30"
  "reasoning:20"
  "other:69"
)

# Deterministic tag + capability pools. Selection is by index mod pool size.
TAG_POOL=(core search indexing planning memory retrieval generation review test bench)
CAP_POOL=(read write analyze synthesize evaluate plan execute report validate summarize)

rm -rf "$OUT_ROOT"
mkdir -p "$OUT_ROOT"

TOTAL=0
for entry in "${CATEGORIES[@]}"; do
  category="${entry%%:*}"
  count="${entry##*:}"
  mkdir -p "$OUT_ROOT/$category"
  i=1
  while [ "$i" -le "$count" ]; do
    idx=$(printf '%03d' "$i")
    name="${category}_agent_${idx}"
    t1="${TAG_POOL[$(( i % ${#TAG_POOL[@]} ))]}"
    t2="${TAG_POOL[$(( (i + 3) % ${#TAG_POOL[@]} ))]}"
    t3="${TAG_POOL[$(( (i + 7) % ${#TAG_POOL[@]} ))]}"
    c1="${CAP_POOL[$(( i % ${#CAP_POOL[@]} ))]}"
    c2="${CAP_POOL[$(( (i + 2) % ${#CAP_POOL[@]} ))]}"
    c3="${CAP_POOL[$(( (i + 5) % ${#CAP_POOL[@]} ))]}"
    cat > "$OUT_ROOT/$category/agent_${idx}.yaml" <<YAML
name: ${name}
version: 1.0.0
description: Synthetic ${category} agent ${idx} for discovery benchmark fixture.
tags:
  - ${t1}
  - ${t2}
  - ${t3}
capabilities:
  - ${c1}
  - ${c2}
  - ${c3}
YAML
    i=$((i + 1))
  done
  TOTAL=$((TOTAL + count))
done

printf 'gen-discovery-fixture: wrote %d files to %s\n' "$TOTAL" "$OUT_ROOT" >&2
