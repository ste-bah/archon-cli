# Banned-Imports Guard

Enforces REQ-FOR-PRESERVE-D8 by greping the archon-cli source tree for regex
patterns that must never be reintroduced.

Spec reference: `project-tasks/archon-fixes/agentshit/02-technical-spec.md`
§1595-1609 ("CI script that greps the workspace for accidental
reintroduction of forbidden patterns").

## Spec divergence at phase-0 (approved 2026-04-11)

The task spec listed 4 initial patterns, one of which was `\bMemoryGraph\b`.
Execution discovered this pattern targets **the archon-cli's own core struct**
(`archon_memory::MemoryGraph` — the embedded CozoDB semantic memory, used
~155 times across 11 crates). That struct is a **validated differentiator**
from claurst and other tools; it is *not* the outer-Archon MemoryGraph MCP
service the spec meant to ban.

Steven explicitly approved **dropping** `\bMemoryGraph\b` from the pattern
list. The three remaining patterns still cover REQ-FOR-PRESERVE-D8 via the
outer MCP tool namespace (`mcp__memorygraph__`) and the file-based fallback
bans. Pattern count therefore diverges from spec validation #6 (expected 4,
actual 3).

## When to run

- Locally before every commit that touches `crates/`, `src/`, or `tests/`.
- In CI (wired by TASK-AGS-007 — not this task).
- After every merge from another branch.

```bash
bash scripts/check-banned-imports.sh
```

Exit `0` = clean. Exit `1` = one or more banned patterns hit; every match is
printed as `BANNED: <pattern>  found at <file:line>`.

## What is scanned

Scan roots: `crates/`, `src/`, `tests/`.
Excluded: `target/`, `tests/fixtures/baseline/`, and the `scripts/`
directory itself.

## Patterns file

`scripts/check-banned-imports.patterns` — one ERE regex per non-comment line.
Every pattern MUST be preceded by a `# REQ-*` comment naming the requirement
it enforces. Later tasks **append**; they do not replace existing entries.

## Path allowlist

`scripts/check-banned-imports.allowlist` — `pattern: repo-relative-path`
tuples, one per non-comment line. A hit matching **both** the pattern and the
exact path is skipped. The allowlist exists for defensive or documentation
references (e.g. bridge code that names the outer MCP tool it replaces, tests
that verify the ban is enforced).

Every allowlist entry MUST cite the REQ it still honours and explain why the
match is legitimate. The allowlist is **not** a release valve — it shrinks
as code is cleaned up.

### Adding a new ban

1. Cite a REQ in a preceding `#` comment.
2. Explain in one line why the ban exists and the correct alternative.
3. Write as portable ERE regex (`grep -E`). No PCRE extensions.

Later tasks **append** to this file; they never replace existing entries.

### Removing a ban

Never, unless the underlying REQ is withdrawn. Removing a ban is a
requirements-level change and must be reviewed as such.

## False positives

If a legitimate match cannot be avoided, either:

1. Restructure the code so the string is constructed at runtime from parts
   that individually do not match the pattern; OR
2. Add a precise `pattern: path` tuple to
   `scripts/check-banned-imports.allowlist` with a REQ-citing comment; OR
3. Move the fixture under `tests/fixtures/baseline/` (already excluded
   from the scan).

Do NOT weaken the regex to allow the false positive.
