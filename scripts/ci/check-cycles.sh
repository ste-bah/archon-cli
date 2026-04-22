#!/usr/bin/env bash
# Fail CI when workspace dependency graph has any circular dependency (SCC > 1).
# Spec: 02-technical-spec.md line 1129 — `cargo depgraph --workspace-only -> cycle==0`
# Implements: AC-OBSERVABILITY-03, TC-TUI-OBSERVABILITY-03, NFR-TUI-MOD-002
set -euo pipefail

DOT_FILE="${DEPGRAPH_OVERRIDE:-/tmp/depgraph.dot}"

if [[ -z "${DEPGRAPH_OVERRIDE:-}" ]]; then
    if ! command -v cargo-depgraph >/dev/null 2>&1; then
        echo "Installing cargo-depgraph..."
        cargo install cargo-depgraph
    fi
    cargo depgraph --workspace-only --depth 1 > "$DOT_FILE"
fi

if [[ ! -f "$DOT_FILE" ]]; then
    echo "ERROR: dependency graph file '$DOT_FILE' not found" >&2
    exit 2
fi

python3 - "$DOT_FILE" <<'PY'
import re
import sys
from collections import defaultdict

dot_path = sys.argv[1]

# Parse DOT edges of the form: "src" -> "dst" ;  or src -> dst ;
edge_re = re.compile(r'"?([^"\s]+)"?\s*->\s*"?([^"\s;]+)"?\s*;?')

nodes = set()
edges = defaultdict(list)
with open(dot_path) as f:
    for line in f:
        m = edge_re.search(line)
        if m and '->' in line:
            src, dst = m.group(1), m.group(2)
            nodes.add(src)
            nodes.add(dst)
            edges[src].append(dst)

# Tarjan's strongly connected components (iterative to avoid recursion limit).
index_counter = [0]
stack = []
on_stack = {}
indices = {}
lowlinks = {}
sccs = []

def strongconnect(start):
    work = [(start, iter(edges.get(start, [])))]
    indices[start] = index_counter[0]
    lowlinks[start] = index_counter[0]
    index_counter[0] += 1
    stack.append(start)
    on_stack[start] = True

    while work:
        v, it = work[-1]
        try:
            w = next(it)
            if w not in indices:
                indices[w] = index_counter[0]
                lowlinks[w] = index_counter[0]
                index_counter[0] += 1
                stack.append(w)
                on_stack[w] = True
                work.append((w, iter(edges.get(w, []))))
            elif on_stack.get(w):
                lowlinks[v] = min(lowlinks[v], indices[w])
        except StopIteration:
            work.pop()
            if lowlinks[v] == indices[v]:
                component = []
                while True:
                    w = stack.pop()
                    on_stack[w] = False
                    component.append(w)
                    if w == v:
                        break
                sccs.append(component)
            if work:
                parent = work[-1][0]
                lowlinks[parent] = min(lowlinks[parent], lowlinks[v])

for n in list(nodes):
    if n not in indices:
        strongconnect(n)

cyclic = [c for c in sccs if len(c) > 1]

# Also treat a self-loop (single-node SCC with self-edge) as a cycle.
for c in sccs:
    if len(c) == 1:
        n = c[0]
        if n in edges.get(n, []):
            cyclic.append(c)

if cyclic:
    print(f"FAIL: {len(cyclic)} cycle(s) detected in workspace dep graph", file=sys.stderr)
    for i, comp in enumerate(cyclic, 1):
        print(f"  cycle #{i}: " + " -> ".join(sorted(comp)), file=sys.stderr)
    sys.exit(1)

print(f"OK: 0 cycles in workspace ({len(nodes)} nodes, {sum(len(v) for v in edges.values())} edges)")
PY
