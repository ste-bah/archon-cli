# Writing a PRD for the Archon workflow engine

A framework for PRDs that are decomposed into a `TASK-*.md` directory and
executed as a generated workflow run, with a dependency graph, single-writer
enforcement, per-task adversarial review, and requirement traceability.

## 0. Which pipeline this is, and when to use it

Archon has two independent routes from a PRD to running code. They write to
different subfolders of the same two roots and must not be mixed inside one
document.

| | Workflow path (this template) | Skills chain |
|---|---|---|
| Author with | `/workflow-prd` then `/workflow-prd-spec` | `/to-prd` then `/prd-to-spec` |
| PRD lands at | `prds/PRD-<NAME>/PRD-<NAME>.md` | `prds/<slug>/PRD.md` |
| Tasks land at | `tasks/PRD-<NAME>/TASK-<DOMAIN>-<NNN>-<slug>.md` | `tasks/phase<N>/task<M>.md` |
| Consumed by | the workflow engine's task universe | `/spec-to-tasks` then `/archon-code` |
| You get | dependency graph, blocks/depends_on reconciliation, declared write coordination, per-task deliverable contracts, requirement traceability | the 50-agent implementation pipeline |

Use the workflow path when you want the run itself to refuse to proceed on a
contradiction — a cycle, an unresolved dependency, a requirement no task
claims, a deliverable that cannot be verified. Use the skills chain when you
want the 50-agent pipeline.

Everything below is the workflow path. If you are writing for the skills
chain, stop and use `ai-agent-prd.md` instead.

## 1. Output location

Write exactly one file:

```
prds/PRD-<NAME>/PRD-<NAME>.md
```

`<NAME>` is SCREAMING-KEBAB-CASE, derived from the product area and a
three-digit sequence — `TRADING-DATA-LAKE-AHDM-001`,
`ALERT-DISPOSITION-002`. The same `<NAME>` names the task directory that
`/workflow-prd-spec` will create at `tasks/PRD-<NAME>/`, and every task file
will carry `prd: PRD-<NAME>`. Pick it once; it is an identifier, not a title.

### 1.1 One consequence of the two-root layout, stated plainly

`archon workflow lint --tasks <dir>` computes its `## requirement coverage`
section by looking for the PRD **beside the task directory** — at
`tasks/PRD-<NAME>.md`. With the PRD under `prds/` that file does not exist, so
that one lint section prints `no PRD found beside …; skipped` and names the
paths it tried. It does not guess, and it does not fall back to scanning for
any markdown file containing requirement IDs.

Nothing else degrades. The authoritative coverage and traceability check is:

```
archon requirements trace --prd prds/PRD-<NAME>/PRD-<NAME>.md --tasks tasks/PRD-<NAME>/
```

which takes both paths explicitly and is unaffected by the layout. Run that,
not the lint section, to answer "is every requirement claimed and proven".

## 2. Number the sections. This is load-bearing.

Every section heading carries a number:

```markdown
## 6. Coverage Matrix

### 6.1 Required universe

### 6.2 Freshness classification
```

Task files cite these numbers in `source_sections: ['6', '6.1', '23']`. That
field is how a reader gets from a task back to the paragraphs that justify
it. Unnumbered sections make the citation unwriteable, and an author who
cannot cite a section writes a task that no longer traces to anything.

Renumbering a section after tasks are written silently invalidates every
citation to it. Append new sections rather than renumbering; if you must
renumber, re-run `/workflow-prd-spec` or fix `source_sections` by hand.

## 3. Requirement bullets: the exact grammar

Requirement IDs are extracted by regex from the rendered document. Two
independent extractors read this file and they do not accept the same thing.
Write the form that satisfies both:

```markdown
- REQ-DL-040: The coverage command MUST classify a cell as covered only when
  its backing dataset meets the declared minimum row count. Violation
  severity: `error` — the coverage artifact fails closed and the run refuses
  the promotion.
```

The rules, each of which is a real failure mode:

- **Start at column 0 with a literal `- `.** The stricter extractor is
  anchored `^- `; a leading space or a `*` bullet makes the requirement
  invisible to `archon requirements trace` while still visible to the lint,
  so the two disagree about how many requirements exist.
- **`REQ-<AREA>-<NNN>` where `<AREA>` is uppercase letters only.** The trace
  extractor accepts `[A-Z]+`; the lint accepts `[A-Z0-9]+`. Letters-only is
  the intersection. `REQ-DL2-040` is counted by one and not the other.
- **Exactly three digits.** `REQ-DL-40` is not a requirement ID.
- **A colon and a single space after the ID**, then the text. The trace
  extractor requires `": "` literally; without it the line is not a
  requirement.
- **One requirement per line, and the ID is the first thing on the line.** An
  ID mentioned mid-paragraph is never extracted. This is deliberate: an
  invisible ID would otherwise pass the coverage check by never being
  counted.
- **Never inside a fenced code block.** The trace extractor skips fenced
  regions, the lint does not, so an ID in an example block inflates the lint's
  denominator and is absent from the trace.
- **Continuation lines are indented exactly two spaces**, carry no bullet
  marker and no heading, and follow with no blank line between. They are
  joined to the requirement text with a single space. A blank line ends the
  requirement.

Group the bullets under the numbered section they belong to. A requirement
list detached from its section is a list the tasks cannot cite.

### 3.1 Per-requirement severity

Severity belongs to the requirement, not to a validation check that happens
to mention it. When severity is attached only to checks, the falsification
scope has to be recovered by phrase-matching prose, and it recovers almost
nothing — on the reference corpus that derivation classified 2 of 93
requirements.

Declare it inline, as the last clause of the requirement text:

```markdown
- REQ-DL-041: Every coverage cell marked available MUST carry captured
  live-fetch provenance. Violation severity: `error` — the cell is reported
  as a gap and the artifact fails closed.

- REQ-DL-042: The text report SHOULD order gaps by instrument then timeframe.
  Violation severity: `warn` — the report is still accepted.
```

What each form actually does today, so you are not guessing:

- ``Violation severity: `error` `` is recognised. The classifier scans the
  requirement text case-insensitively for `` `error` ``, `status=failed`,
  `fail closed`, or `fails closed`, and the first hit sets `Severity::Error`
  and records the matched phrase as the evidence for that classification. The
  backticks are part of the literal — `severity: error` unbacktick'd does not
  match on its own.
- ``Violation severity: `warn` `` is **not** recognised by any phrase and the
  requirement is recorded as `Unclassified`. It is not silently promoted to
  error and it is not dropped; it is reported as unclassified. Write it
  anyway — it is the declaration a future classifier reads, and an
  unclassified requirement that says `warn` is unambiguous to a human
  reviewer, which an unclassified requirement that says nothing is not.

Write the severity clause on the requirement line or on a two-space-indented
continuation of it. A severity sentence in the following paragraph is not
part of the requirement text and is not scanned.

## 4. Sections this document must contain

### 4.1 Numbered content sections

Whatever the product needs — problem statement, users, architecture, data
contracts, thresholds, rollout. Number them all. Put the requirement bullets
under them.

### 4.2 `## Hard Rules` and/or `## Constraints`

These two heading names are harvested verbatim by the workflow's PRD intake
and injected into every agent prompt for the run. Heading match is
case-insensitive and the section runs until the next heading at the same or
higher level. Every non-blank line in the section becomes a rule, whether or
not it is a bullet, so do not put prose commentary here.

Put here only what must hold for every task: banned dependencies, forbidden
directories, the "no resampling" rule, the secrets policy. A rule buried in a
numbered section is documentation; a rule here is in the prompt.

### 4.3 A decomposition section

List the intended tasks in dependency order, with the requirement IDs each is
expected to claim. `/workflow-prd-spec` reads this to seed the decomposition,
and a reviewer reads it to check the graph before any code is written.

If you name specific `TASK-<DOMAIN>-<NNN>` ids here, every one of them must
exist as a file after decomposition. When the PRD path is handed to a
generated run as evidence, the engine harvests every task id mentioned in the
PRD and refuses the run naming any id with no matching `TASK-*.md` file:
`references TASK-XYZ-010 but no matching TASK-*.md file was found`. A
renumbered or deleted task leaves a dangling citation that fails the run, not
a stale sentence.

### 4.4 A traceability section

A table of requirement ID to intended task id. Redundant with `implements:`
by design: the table is the author's intent, `implements:` is the task's
claim, and the coverage check reports where they differ.

## 5. Writing requirements the engine can verify

### 5.1 Name the artifact and the command

A requirement that describes a behaviour with no artifact and no command
cannot be promoted past "candidate" no matter how well it is implemented.
Promotion needs two facts to line up: a declared verifier command that
actually ran and passed, and a recorded read of the anchored file during that
run. So write requirements that name both:

```markdown
- REQ-DL-060: `archon trading data coverage --universe trading-core-v1` MUST
  write `.archon/trading-lab/data/coverage/latest.json` conforming to §23.
  Violation severity: `error` — the command exits non-zero and writes no
  artifact.
```

The task that implements it can then declare
`cargo test -p archon-trading coverage_matrix` under `## Focused Tests` and
`crates/archon-trading/src/data_lake.rs` under
`## Files Expected to Change`, and the trace has everything it needs.

### 5.2 Artifact contract rules the PRD must respect

Tasks declare `deliverable_contracts` naming an `artifact_path`. Three PRD-side
rules follow from how those are verified:

- **A path containing `<...>` is a template and needs an instance binding.**
  If the PRD specifies an artifact family — `data/<dataset-id>/summary.json`
  — say in the PRD which source collection enumerates the instances and which
  field of each record names the artifact. Without that, the task can only
  fall back to a minimum count.
- **A minimum of zero is not a gate.** If the PRD says "produce a summary for
  each dataset" and there are no datasets, a `min_instances: 0` contract
  passes with zero files. State the floor in the PRD: "at least one", "one per
  entry in the registry". A floor the PRD does not state becomes a floor the
  task sets to zero.
- **A typed verifier takes one concrete path.** If the PRD wants a typed
  verifier command for an artifact, that artifact must be a single named file,
  not a family. Deciding this in the PRD avoids a task that declares both and
  is refused.

### 5.3 Concurrently written files

When two tasks must append to the same file, the PRD must carry the
coordination requirement as a normative requirement in its own right:

```markdown
- REQ-REG-010: Appends to `.archon/registry/index.jsonl` MUST be
  single-writer-safe: each appender acquires an exclusive advisory lock,
  writes one complete record, and releases. Violation severity: `error` —
  interleaved partial records fail the registry parse and the run refuses the
  artifact.
```

The task-side `shared_append_target_files:` declaration tells the write
coordinator to stop serialising those tasks against each other. It is an
assertion that the appends are coordinated; it does not implement the
coordination. Without a normative requirement like the one above, declaring
the field removes the scheduler's protection and puts nothing in its place.

## 6. Quality gate before decomposition

Do not hand the PRD to `/workflow-prd-spec` until all of these hold. Each maps
to a check that runs later, so a failure here is a failure there.

1. Every section is numbered, and no section has been renumbered since any
   task was written.
2. Every requirement bullet starts at column 0 with `- REQ-<LETTERS>-<NNN>: `.
3. Every requirement carries a `Violation severity:` clause.
4. No requirement ID appears inside a fenced code block.
5. Every requirement names either an artifact path, a command, or a file that
   changes — something a task can anchor to.
6. Every requirement is assigned to at least one task in the decomposition
   section. An unassigned requirement is a decomposition gap and will be
   reported as one.
7. `## Hard Rules` contains only rules, one per line.
8. Every `TASK-<DOMAIN>-<NNN>` id named in the document is one the
   decomposition will actually create.
9. No "should work", "probably", "later", "TBD", or "best effort" anywhere.
   If something is genuinely undecided, write it as an open question with the
   decision owner and state what the system does until it is decided —
   including which way it fails.

## 7. Worked fragment

```markdown
# PRD-TRADING-DATA-LAKE-AHDM-001 — Trading Data Lake

## 1. Problem

...

## 6. Coverage Matrix

### 6.1 Required universe

The trading-core-v1 universe is ES, NQ, SPY, QQQ, BTCUSDT, ETHUSDT across
timeframes 1W, 1D, 240, 60, 15 — thirty cells.

- REQ-DL-040: `archon trading data coverage --universe trading-core-v1` MUST
  emit `.archon/trading-lab/data/coverage/latest.json` with one cell per
  instrument-timeframe pair. Violation severity: `error` — the command exits
  non-zero.
- REQ-DL-041: A cell MUST be marked available only when its backing dataset
  holds at least 200 rows of native-interval candles. Violation severity:
  `error` — the cell is reported as a gap with the observed row count.
- REQ-DL-042: Gaps MUST carry an exact machine-readable reason.
  Violation severity: `error` — a gap with an empty reason fails closed.

### 6.2 Freshness

- REQ-DL-043: A current snapshot older than five minutes MUST be classified
  stale. Violation severity: `error` — stale data cannot satisfy coverage.

## Hard Rules

- No production candle resampling.
- No hardcoded provider credentials.
- Changed and new files stay under 500 lines.

## 15. Decomposition

| Task | Claims |
|------|--------|
| TASK-TDL-080 Coverage matrix command | REQ-DL-040, REQ-DL-041, REQ-DL-042 |
| TASK-TDL-041 Snapshot freshness | REQ-DL-043 |
```

## 8. After writing

Print the path you wrote to, then tell the user the next command:

```
/workflow-prd-spec prds/PRD-<NAME>/PRD-<NAME>.md
```

Do not print the PRD body into the conversation.
