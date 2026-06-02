# Dynamic workflow cookbook

Use dynamic workflows when the task is too large for one chat turn but does not
fit an existing static pipeline. They are best for ad hoc orchestration: repo
audits, design reviews, source-backed analysis, research scouting, migration
planning, and other jobs where Archon needs a durable plan, several agents,
fan-out/fan-in, restartable stages, and compact progress.

Dynamic workflows are not a replacement for the audited static lanes:

- use `/archon-code` for production coding pipelines
- use `/archon-research` for full PhD-style research papers
- use `/gametheory` for strategic/game-theory analysis

Use `/workflow` when the task shape is custom.

## Plan first

```bash
archon workflow plan "Audit this repository deeply. Use critics and produce a report."
```

Inside the TUI:

```text
/workflow plan Audit this repository deeply. Use critics and produce a report.
```

Planning prints the `WorkflowSpec` YAML and does not execute any stages. In the
TUI, planning uses the active provider, validates the generated schema, and
falls back to the heuristic planner if repair fails. Check for the important
safety properties before running:

- no hard-coded provider/model fields
- fan-out stages have a reducer
- dangerous tool stages are absent or policy-gated
- max parallelism is reasonable for the active provider

## Run

```bash
archon workflow run "Audit this repository deeply. Use critics and produce a report."
```

Inside the TUI:

```text
/workflow run Audit this repository deeply. Use critics and produce a report.
```

The run is written to `.archon/workflows/<run-id>/`. The parent transcript gets
compact progress instead of raw agent dumps. TUI runs use the active provider
configured for the session and emit workflow-scoped rows in Agent Activity.

## Real-world example: repository audit

TUI:

```text
/workflow run Audit this repository for regressions introduced in the latest branch. Use discovery, fan-out reviewers, adversarial verification, and a final evidence-weighted report. Do not run destructive commands. Do not run the full test suite unless the workflow plan justifies it.
```

What to expect:

- `discover` identifies relevant modules/files
- `review` fans out per module or concern
- `synthesize` merges findings into one report
- `quality` checks whether the report is usable
- artifacts land under `.archon/workflows/<run-id>/artifacts/`

If the workflow plan looks too broad, run `plan` first, inspect the YAML, then
run with a tighter task.

## Real-world example: research scout

TUI:

```text
/workflow run Scout sources for an implementation-ready PRD about provider-neutral workflow orchestration. Use my docs store where relevant, web search for current external references, contradiction review, and a final source-backed design brief.
```

Use this when you want source gathering and design synthesis but not the full
`/archon-research` paper pipeline. If the output needs to become a formal paper,
switch to `/archon-research`.

## Real-world example: migration plan

TUI:

```text
/workflow run Create a migration plan to move the document index queue to a faster backend. Split discovery across storage, embedding, API, TUI, and docs concerns. Produce implementation phases, risks, tests, and rollback steps.
```

Good workflow outputs here are not just prose. Inspect artifacts for:

- stage-specific findings
- dissent or failed-stage summaries
- exact files/modules to change
- acceptance tests
- rollback notes

## Inspect and resume

```bash
archon workflow list
archon workflow status <run-id>
archon workflow resume <run-id>
```

If a single stage output is bad, rewind only that stage:

```bash
archon workflow restart-agent <run-id> <stage-id>
archon workflow resume <run-id>
```

Inside the TUI, use the same arguments after `/workflow`.

`/workflow list` opens the Dynamic Workflows inspection view. `/workflow status
<run-id>` opens stage-level rows so failed, skipped, and retried stages remain
visible before you decide whether to restart a single stage.

The web workbench also has a **Workflows** page. Start the web UI, open
`#/workflows`, and check:

- recent durable workflow runs
- accepted/failed stage counts
- sanitized event previews
- available policy-gated controls

The backing endpoint is `GET /api/workflows/summary`.

The page also opens run detail, follows `GET /api/workflows/<run-id>/stream`
for live sanitized events, and submits policy-gated controls through
`POST /api/workflows/control`.

## Restart bad output

If one stage produced garbage, do not rerun the whole workflow:

```text
/workflow status <run-id>
/workflow restart-agent <run-id> <stage-id>
/workflow resume <run-id>
```

`restart-agent` rewinds that stage and downstream dependent state while keeping
accepted upstream work. Use this when a reviewer hallucinated, a source scan was
too shallow, or the reducer produced a weak synthesis.

If a failed quality gate is acceptable and you want to continue without
rewriting the stage, force-accept it with a rationale:

```text
/workflow force-accept <run-id> <stage-id> accepted after manual source check
/workflow resume <run-id>
```

Forced acceptance writes an audit event and keeps the stage out of durable
memory unless it already has accepted artifacts. Use restart for bad work; use
force-accept only for reviewed work that failed a conservative gate.

## Recover after interruption

If Archon exits mid-run:

```text
/workflow list
/workflow status <run-id>
/workflow resume <run-id>
```

Resume uses durable state under `.archon/workflows/<run-id>/`, not transcript
text. Accepted stages are skipped; pending or restarted stages continue.

## Save a reusable template

```bash
archon workflow save <run-id> repo-deep-audit
```

Saved templates are sanitized: run ids, hard-coded models, provider-private
payloads, credentials, and stale artifact hashes are not carried forward.

## Learning output

After a run finishes, inspect:

```bash
ls .archon/workflows/<run-id>/learning
```

`records.jsonl` contains every stage outcome. `durable-memory.jsonl` contains
only accepted stages with artifacts. This separation is intentional: failed,
forced, or unverified stages stay auditable without poisoning durable learning.

Direct handoff files are also written for learning consumers:

```text
adapter-sona.jsonl
adapter-rlm.jsonl
adapter-reflexion.jsonl
adapter-reasoning-bank.jsonl
adapter-jepa.jsonl
adapter-world-model.jsonl
```

These records give each subsystem a direct workflow trace to consume without
parsing the generic audit ledger.

## Save and reuse a workflow

After a good run:

```text
/workflow save <run-id> repo-deep-audit
```

Saved templates remove run ids, provider-private payloads, credentials, stale
artifact hashes, and temporary paths. Reuse templates for repeatable jobs, but
still inspect the plan when the task or project changes.

## Safety checklist

Before running a generated workflow against real tools or a large repo, confirm:

- no `provider` or `model` fields are hard-coded in stages
- `max_parallelism` is sane for the active provider
- fan-out stages have a downstream reducer
- tool stages do not request dangerous commands without a policy gate
- expected artifacts are stored rather than dumped into chat
- failed or forced stages are visible in `status` before accepting output

## When not to use it

Use the static pipelines when the lane already exists:

- `/archon-code` for production coding work
- `/archon-research` for full research papers
- `/gametheory` for strategic/game-theory analysis

Dynamic workflows are for generated orchestration. The static pipelines remain
the audited production lanes for `/archon-code`, `/archon-research`, and
`/gametheory`.
