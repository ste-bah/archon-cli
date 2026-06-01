# Pipeline Rewind Cookbook

Use pipeline rewind when a coding or research pipeline has accepted bad agent
outputs and a normal resume would carry that bad evidence forward. For a run
started with `/archon-research`, rewind and resume it through `/pipeline`; do
not start a second `/archon-research` unless you intentionally want a brand-new
research bundle.

Rewind is different from resume:

- `resume` continues from the next unfinished agent.
- `rewind` quarantines completed agent records from a chosen point onward, then
  the next resume re-runs those agents.

## When To Use It

Use rewind when:

- a quality gate was force-accepted but the accepted output is wrong;
- an early writer or reviewer used the wrong source set;
- downstream chapters, citations, appendices, or final artifacts are poisoned by
  earlier bad outputs;
- you need to restart from the first bad agent without deleting the whole
  audited bundle.

Do not use rewind for a simple crash, network failure, or interrupted run where
the completed agent outputs are still trusted. Use `resume` for that.

## What Rewind Does

For a bundle at:

```text
<workdir>/.archon/pipelines/<session-id>/
```

`archon pipeline rewind`:

- moves active audited agent records from the rewind point onward out of
  `agents/`;
- stores them under `rewound/<timestamp>-keep-<n>/agents/` for audit history;
- moves the stale primary prompt, output, attempt, and known research artifact
  files into the same quarantine tree so they are not overwritten by the
  regenerated run;
- moves stale final research exports out of `exports/` so an old paper is not
  mistaken for the regenerated result;
- recomputes `state.json` from the kept audited records;
- clears final completion metadata and marks the bundle resumable;
- appends a `run_rewound` event to `audit.log`.

The important part: resume hydrates completed work from active `agents/*.json`.
Once bad agent records are quarantined, the next resume cannot treat those bad
agents as completed.

## Pick The Rewind Point

First inspect the bundle:

```bash
archon pipeline inspect <session-id>
archon pipeline verify <session-id> --write-report
```

Inside the TUI, use the slash equivalents:

```text
> /pipeline inspect <session-id>
> /pipeline verify <session-id> --write-report
```

Find the earliest accepted output that is wrong. Rewind to that agent, not to
the final agent. If `conclusion-writer` is the first bad output, rewind to
`conclusion-writer` so it and every downstream agent are regenerated:

```bash
archon pipeline rewind <session-id> \
  --to-agent conclusion-writer \
  --reason "quarantine contaminated writer and downstream review outputs"
```

You can also target by ordinal:

```bash
archon pipeline rewind <session-id> --to-ordinal 32
```

Or by exact keep count:

```bash
archon pipeline rewind <session-id> --keep-agents 31
```

Prefer `--to-agent` during incident recovery because it is harder to miscount.

## TUI Flow

Yes, pipeline rewind works from the TUI. For a `/archon-research` run, the
recovery flow is:

```text
> /pipeline inspect <session-id>
> /pipeline rewind <session-id> --to-agent conclusion-writer --reason "bad accepted output"
> /pipeline verify <session-id> --write-report
> /pipeline resume <session-id>
```

Current TUI behavior is intentionally split:

- `/pipeline rewind ...` is a quick audited state mutation mirrored through the
  pipeline command surface. It prints its result in the transcript.
- `/pipeline resume ...` uses the TUI-aware in-process resume path, so renewed
  agent launches appear in Agent Activity with provider, model, status, and
  tool details.

That means the normal TUI recovery flow is: rewind first, then resume.

The resumed bundle remains the original `/archon-research` bundle. Archon uses
the persisted pipeline type in the audited state to continue the research
pipeline at the correct agent.

## After Rewind

Resume the bundle:

```bash
archon pipeline resume <session-id>
```

Or in the TUI:

```text
> /pipeline resume <session-id>
```

If the bundle was already running, stop or abort that run before rewinding. A
rewind is an audited state change and should not race a live pipeline writer.

## Force Quality Gate Is Not A Substitute

`--force-quality-gate` exists for manually reviewed low-scoring attempts that
are still acceptable. It is not a cleanup tool for contaminated outputs.

Use:

- `resume` for trusted completed outputs;
- `resume --force-quality-gate` for a reviewed threshold miss;
- `rewind` when accepted outputs are bad and must be regenerated.

Semantic citation hard failures remain non-bypassable. If citation evidence is
missing or inconsistent, fix the source problem or rewind to the agent that
introduced the bad evidence.

## Verify The Recovery

After resume finishes, inspect the bundle again:

```bash
archon pipeline verify <session-id> --write-report
archon pipeline inspect <session-id>
```

Check:

- the bad agent records and their old prompt/output artifacts are under
  `rewound/`;
- active `agents/` contains regenerated records from the rewind point onward;
- the final artifacts were produced after the rewind event;
- final citations and appendices cite the intended source set.
