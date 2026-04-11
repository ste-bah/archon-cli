# Agent Discovery Fixtures

This directory holds YAML fixtures for agent discovery tests and
benchmarks (REQ-DISCOVERY-002/004, NFR-PERF-002).

## Why the `generated/` subtree is not committed

The NFR-PERF-002 benchmark needs 300 near-identical agent metadata files
to exercise the discovery scan. Committing 300 YAMLs produces ~300 KB of
churn on every regeneration and buries real fixture changes under bulk
edits. Instead, a deterministic generator writes them on demand:

```bash
bash scripts/gen-discovery-fixture.sh
```

The script is:

- **Deterministic** — no timestamps, no RNG; the same inputs produce
  byte-identical output on every run.
- **Idempotent** — wipes `generated/` before writing, so reruns never
  leave stale files behind.
- **Cheap** — pure shell + heredocs; no cargo build.

`generated/` is ignored via `tests/fixtures/agents/.gitignore`, so it
never accidentally enters a commit.

## Layout

```
generated/
  custom/           11 agents
  development/      60 agents
  coding-pipeline/  50 agents
  core/             30 agents
  analysis/         30 agents
  hive-mind/        30 agents
  reasoning/        20 agents
  other/            69 agents
                   ----
                    300 total
```

Each file is `agent_NNN.yaml` (`NNN` zero-padded) with the minimum
REQ-DISCOVERY-004 metadata:

```yaml
name: <category>_agent_NNN
version: 1.0.0
description: Synthetic <category> agent NNN for discovery benchmark fixture.
tags:
  - <tag>
  - <tag>
  - <tag>
capabilities:
  - <cap>
  - <cap>
  - <cap>
```

Tags and capabilities are chosen deterministically from a fixed pool
via the agent index, so running the generator on two hosts produces the
same bytes.

## How phase-3 tests consume the fixture

The discovery benchmark (phase-3, REQ-DISCOVERY-001..008) invokes this
generator in its `build.rs` or test `setup` before scanning. The
contract is: **the generator must be safe to run repeatedly and must
finish in well under a second**. Keep it shell + heredocs.

## Non-generated fixtures

Hand-written edge-case fixtures (malformed YAML, schema violations,
version collisions) are phase-3's responsibility and live in sibling
directories when they are added. They are NOT covered by this
`.gitignore` — only `generated/` is excluded.
