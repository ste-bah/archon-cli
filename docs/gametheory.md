# Game-Theory Evidence Pipeline

The game-theory pipeline classifies a strategic situation, routes it through a
YAML specialist graph, executes selected specialists, and persists a final
report with provenance. It is exposed through CLI commands, a `/gametheory`
slash umbrella, and eight agent-callable tools.

## CLI

Current `archon gametheory --help` surface:

| Command | Purpose | Important flags |
|---|---|---|
| `run <situation>` | Run classify, route, specialists, final report | `--classify-only`, `--spec-path`, `--debug-memory`, `--budget`, `--max-concurrent`, `--style`, `--enable-tier11` |
| `list-runs` | List persisted runs | none |
| `show <run-id>` | Show full run details | none |
| `status [run-id]` | Show one run or aggregate status counts | optional run id |
| `inspect <artifact-id>` | Inspect run, specialist, section, fingerprint, routing, or report artifact | artifact id forms include `fingerprint:<run-id>` and `specialist:<run-id>:<agent>` |
| `inspect-fingerprint <run-id>` | Inspect Tier 1 fingerprint | none |
| `inspect-routing <run-id>` | Inspect routing decision | none |
| `replay <run-id>` | Replay from persisted fingerprint | `--spec-path`, `--reclassify`, `--rerun-specialist <key>` |
| `resume <run-id>` | Resume interrupted `InProgress` run from checkpoints | `--spec-path` |
| `list-agents` | List curated specialists | `--tier N` |
| `specimens` | List or ingest known-fingerprint library | `--filter axis=value`, `--ingest` |

Example:

```bash
archon gametheory run "Assess this plugin marketplace design" \
  --budget 20 \
  --max-concurrent 4 \
  --style executive \
  --debug-memory
```

## Source of truth

The pipeline persists real state into Cozo relations including:

| Relation | Meaning |
|---|---|
| `gt_runs` | run id, situation, timestamps, status, cost |
| `gt_fingerprints` | Tier 1 classification fingerprint |
| `gt_routing_decisions` | enabled and skipped specialists |
| `gt_specialist_outputs` | per-agent outputs, status, cost |
| `gt_sections` | report section drafts |
| `gt_final_reports` | final assembled report |
| `gt_run_checkpoints` | resume/replay checkpoints |
| `gt_specimen_library` | lazy-loaded known fingerprints |

Full State Verification should read these through CLI inspection commands:

```bash
archon gametheory list-runs
archon gametheory status <run-id>
archon gametheory inspect-fingerprint <run-id>
archon gametheory inspect-routing <run-id>
archon gametheory inspect specialist:<run-id>:<agent-key>
```

## Slash commands

Interactive TUI users get one umbrella command:

| Slash form | Equivalent intent |
|---|---|
| `/gametheory run <situation>` | Start an async game-theory run |
| `/gametheory status [run-id]` | Show status |
| `/gametheory inspect <artifact-id>` | Inspect an artifact |
| `/gametheory list-runs` | List persisted runs |
| `/gametheory show <run-id>` | Show run details |
| `/gametheory replay <run-id> [--reclassify|--rerun-specialist <key>]` | Replay a run |
| `/gametheory list-agents [--tier N]` | List specialists |
| `/gametheory specimens [--filter axis=value] [--ingest]` | Inspect specimen library |

## Agent tools

`archon-tools` registers these game-theory tools when a `GameTheoryExecutor` is
installed:

| Tool | Inputs |
|---|---|
| `GameTheoryRun` | `situation`, optional `budget_usd`, `max_concurrent`, `style` |
| `GameTheoryStatus` | optional `run_id` |
| `GameTheoryListAgents` | optional `tier` |
| `GameTheorySpecimens` | optional `filter`, `ingest` |
| `GameTheoryInspect` | `artifact_id` |
| `GameTheoryReplay` | `run_id`, `reclassify`, optional `rerun_specialist` |
| `GameTheoryClassify` | `situation` |
| `GameTheoryCallSpecialist` | `run_id`, `agent_key` |

These tools call the same persisted machinery as the CLI. They should not print
canned text without writing or reading the expected Cozo state.
