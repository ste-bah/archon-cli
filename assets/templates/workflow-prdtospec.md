# Decomposing a PRD into a workflow task directory

A framework for turning a PRD written with `workflow-prd` into the flat
`TASK-*.md` directory that the Archon workflow engine walks, plans, and
executes.

## 0. What this produces, and what reads it

This is the second half of the workflow path. It is not the skills chain: it
does not write `tasks/INDEX.md`, it does not write `tasks/phase<N>/task<M>.md`,
and `/spec-to-tasks` and `/archon-code` do not read what it produces. If you
want those, use `prdtospec.md` instead.

What reads this output:

- **The task universe extractor**, which walks the directory when a generated
  run's task text names it, parses every file, reconciles `depends_on` and
  `blocks` into one graph, validates statuses, and refuses the run on any
  contradiction.
- **`archon workflow lint --tasks <dir>`**, which lowers the same directory
  into the topology IR and reports write conflicts, fake edges, and
  requirement coverage.
- **`archon requirements trace --prd <prd> --tasks <dir>`**, which walks
  requirement IDs from the PRD to the tasks claiming them to the commands and
  files that prove them.

Every rule below fails closed in at least one of those three. None of them
degrade quietly.

## 1. Output layout

```
tasks/PRD-<NAME>/
├── TASK-<DOMAIN>-001-<slug>.md
├── TASK-<DOMAIN>-010-<slug>.md
├── TASK-<DOMAIN>-020-<slug>.md
└── TASK-<DOMAIN>-140-<slug>.md
```

`<NAME>` is the PRD id from the PRD's own filename — the PRD at
`prds/PRD-TRADING-DATA-LAKE-AHDM-001/PRD-TRADING-DATA-LAKE-AHDM-001.md`
decomposes into `tasks/PRD-TRADING-DATA-LAKE-AHDM-001/`.

The rules are mechanical:

- **One task per file.** There is no multi-task file format.
- **Flat.** Discovery is a single non-recursive read of the directory for
  names matching `TASK-*.md`. A task file one level deeper is not found at
  all — not warned about, not partially loaded, not found. A perfectly
  formatted task in a subdirectory contributes nothing. A directory with no
  matching file is refused naming the directory.
- **Nothing else in the directory needs to move.** Files not matching
  `TASK-*.md` are ignored, so a `README.md` beside the tasks is harmless.

### 1.1 The filename carries the id

`TASK-<DOMAIN>-<NNN>-<slug>.md`, where the id is the first three
dash-separated parts of the stem:

- `TASK` literally.
- `<DOMAIN>` — uppercase letters and digits only, **no internal hyphen**.
  `TDL` is a domain; `DATA-LAKE` is not, because the id would then have four
  parts and be unreadable.
- `<NNN>` — exactly three digits.
- `<slug>` — kebab-case, as many parts as you like; it is not part of the id.

`TASK-TDL-010-registry-schema-v1.md` yields `TASK-TDL-010`. A filename no id
can be read from is refused naming the file.

Number in tens (`001`, `010`, `020`, …) so a task can be inserted later
without renumbering. Renumbering breaks every `depends_on`, `blocks`, and PRD
citation pointing at the old id.

### 1.2 `task_id` inside the file must equal the id from the filename

A mismatch is refused naming both:

```
task_id 'TASK-TDL-011' does not match filename task id 'TASK-TDL-010' in <path>
```

Neither one silently wins. Rename the file and the field together.

## 2. The YAML block

The **first** fenced block in the file, fenced ```` ```yaml ````, immediately
after the `# ` title. Not `---` front matter: the workflow parser accepts
`---` but `archon requirements trace` reads only a ```` ```yaml ```` fence and
would fall back to scanning the whole document. Use the fence.

A file with no YAML block is refused naming the file and listing every key it
should have declared. A block that is not a YAML mapping is refused. An
unreadable `deliverable_contracts` value is refused naming the file and
quoting the parse error.

```yaml
task_id: TASK-TDL-080
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Coverage Matrix Command
workstream: W2 Providers + Coverage
complexity: large
status: blocked
depends_on: ['TASK-TDL-040', 'TASK-TDL-050']
blocks: ['TASK-TDL-090', 'TASK-TDL-100']
source_sections: ['6', '6.1', '23']
implements: [REQ-DL-040, REQ-DL-041, REQ-DL-042]
required_env_keys: [OPENBB_API_URL, POLYGON_API_KEY]
required_tools: [data_get_ohlcv]
shared_append_target_files: []
deliverable_contracts: []
```

### 2.1 Required keys

Ten keys must be **present** in every task file. Presence, not
non-emptiness — `[]` is a valid declaration and a meaningful one.

```
task_id  title  complexity  status  depends_on  blocks
implements  required_env_keys  required_tools  deliverable_contracts
```

A file missing any of them is refused with the file path and the missing key
names:

```
task file <path> is missing required key(s): implements, blocks
```

This is why `[]` matters. `required_tools: []` says "this task needs no
tools"; omitting the key says nothing, and the engine refuses to guess which
you meant.

### 2.2 The other five keys

`prd`, `domain`, `workstream`, `source_sections` are informational to a run —
they are not required and a run does not fail without them. Declare them
anyway: `prd` is read by the coverage lint to locate the PRD, and
`source_sections` is the only path from a task back to the paragraphs that
justify it. `shared_append_target_files` is optional and empty by default;
see §6.

### 2.3 Key semantics

| Key | Value |
|---|---|
| `task_id` | Canonical id, matching the filename. Must be a string. |
| `title` | Human title. Overrides the `# ` heading when both are present. |
| `complexity` | `small` \| `medium` \| `large`. Free-form to the parser; keep to these three. |
| `status` | `pending` \| `blocked` \| `in_progress` \| `done`. An unclassifiable value is a hard failure. `blocked` with an empty `depends_on` **and** no inherited blocker is a hard failure — a task blocked by nothing is an authoring error, not a state. |
| `depends_on` | Task ids this task waits on. Every id must resolve to a task in the directory or the run is refused naming the file and the unresolved id. Short aliases (`T040`) resolve; full ids are clearer. |
| `blocks` | See §4. |
| `implements` | See §3. |
| `required_env_keys` | Environment variable names. Unioned with `.archon/project.json`. |
| `required_tools` | Tool names. Unioned with the project manifest's `required_tools` and every `tool_bundles` entry. |
| `deliverable_contracts` | See §5. |

## 3. `implements:` — the requirement claim

Always declared. A single-line flow sequence, unquoted IDs:

```yaml
implements: [REQ-DL-040, REQ-DL-041, REQ-DL-042]
```

A block sequence is a hard error in `archon requirements trace`:

```
`implements:` must be a single-line flow sequence like `implements: [REQ-DL-020]`
```

even though the workflow parser would accept it. Use the flow form.

For an audit, review, or readiness task that implements no requirement of its
own, declare it empty:

```yaml
implements: []
```

That is a claim — "this task satisfies no PRD requirement" — and it is the
right claim for a review task. Omitting the key entirely is refused.

Two checks run off this field, both pure set operations:

- **Every ID a task cites must be defined in the PRD.** An ID no PRD defines
  is a typo or a reference to a requirement that was renumbered or deleted
  under the task. Reported with the citing task named.
- **Every requirement the PRD defines must be claimed by at least one task.**
  An unclaimed requirement is a decomposition gap: either a task's
  `implements:` is missing an ID, or the work is undecomposed and needs a
  task.

Distribute claims honestly. Listing every requirement on every task makes the
coverage check pass and makes the trace meaningless.

## 4. `depends_on` and `blocks`

Both are parsed, both resolve against the same alias table, and both are
folded into one graph before cycle detection runs: `A blocks B` is merged into
`B.depends_on`. Declare whichever direction is natural where you are standing;
you do not need to mirror every edge by hand.

Three contradictions are refused by name rather than being folded into a
cycle the detector would then report as a graph shape instead of the authoring
mistake it is:

```
task TASK-X-010 declares that it blocks itself in <path>
task TASK-X-010 both blocks and depends_on TASK-X-020 in <path>
tasks TASK-X-010 and TASK-X-020 each declare that they block the other (<path> / <path>)
```

A real cycle in the merged graph is refused with the full path through it,
each task named with its file.

### 4.1 Ordering-only dependencies are legitimate

A dependency where the upstream task produces no artifact the downstream task
consumes is a **sequencing** edge — "do the audit before the migration" — and
the lint reports it as such. That report is information, not a defect.

**Do not fabricate a deliverable contract to silence it.** An invented
artifact path turns a truthful ordering edge into a false dataflow claim, and
then a gate exists that nothing produces. Leave the edge as it is and let it
be reported as ordering-only.

## 5. `deliverable_contracts`

Each contract names something the task is contracted to produce and how it is
checked. Two fields are required by the schema; omitting either makes the
whole block unreadable and refuses the file.

```yaml
deliverable_contracts:
  - kind: coverage_matrix
    artifact_path: .archon/trading-lab/data/coverage/latest.json
    typed_verifier_command: archon trading data verify-coverage {artifact_path}
```

- `kind` — required. A short snake_case name for what this is.
- `artifact_path` — required. The path the task writes.

### 5.1 Distinct `kind` for create versus append

When one task creates a file and another appends entries to it, give them
different kinds:

```yaml
# TASK-X-010 — creates the registry
- kind: x_registry
  artifact_path: .archon/registry/index.json

# TASK-X-020 — appends one entry to it
- kind: x_registry_entry
  artifact_path: .archon/registry/index.json
```

Same path, different contract. Reusing one kind for both makes the two
indistinguishable in the graph, and the creation gate and the append gate
become the same gate.

### 5.2 Templated artifact paths need an instance binding

A path containing a `<...>` segment names a family, not a file. The verifier
opens paths literally, so an unexpanded template can neither pass nor fail —
which is a gate that does nothing. Four rejections, all fail-closed:

1. **A templated `registry_path` or `instance_source_path` is refused.** Those
   are inputs the verifier opens literally and no binding expands them.
   Declare a concrete path.
2. **A templated `artifact_path` with `required_universe: true` is refused.**
   A required-universe contract names one enumerated artifact.
3. **A templated `artifact_path` with a `typed_verifier_command` is refused.**
   A typed verifier is handed one concrete path and cannot expand a template.
   If you need a typed verifier, name a single concrete file.
4. **A templated `artifact_path` with no binding at all is refused**, naming
   the unexpanded token.

To make a templated path checkable, declare **either** the full source
binding — all three of `instance_artifact_field`, one of
`instance_source_path` / `registry_path`, and one of
`instance_source_records_field` / `registry_records_field` — so every instance
is named by a source record:

```yaml
- kind: dataset_summary
  artifact_path: .archon/data/<dataset-id>/summary.json
  instance_source_path: .archon/registry/index.json
  instance_source_records_field: datasets
  instance_artifact_field: summary_path
  min_instances: 1
```

**or** declare `min_instances: 1` or higher, which makes the expansion a claim
that can fail.

### 5.3 `min_instances: 0` is a vacuous gate

Zero matches satisfy a floor of zero. A contract that says "produce a summary
per dataset" with `min_instances: 0` passes when nothing was produced. That is
a defect that has shipped. `min_instances` defaults to 0 when the key is
absent, and a non-integer value falls back to 0, so an absent or malformed
floor is a silently vacuous one.

Set a floor you are willing to defend. If the true floor genuinely depends on
the input, use the source binding in §5.2 instead — then the floor comes from
the records and cannot be understated.

## 6. `shared_append_target_files`

Declare a path here when another task writes the same file concurrently:

```yaml
shared_append_target_files: ['.archon/registry/index.jsonl']
```

What this does: the write coordinator stops scheduling the declaring tasks
against each other for that path. Declaring it is an **assertion** that the
write is coordinated and atomic.

What it does not do: make the write coordinated or atomic. It removes the
scheduler's serialisation and puts nothing in its place. The PRD must carry
the locking and atomicity rule as a normative requirement, and the task must
implement it. Declaring the field without both is strictly worse than not
declaring it.

Leave it absent or `[]` unless a second task really does write the path. A
path is exclusive unless a task names it here, so nothing becomes
concurrently written by omission.

## 7. `## Focused Tests` — runnable commands, not descriptions

**This is the highest-value rule in this document.**

A requirement is only proven satisfied when a named command passes **and** the
ambient trace shows that run read the anchored code. A prose bullet supplies
neither, so it can never promote a requirement past "candidate".

On the reference 17-task corpus, every focused-test entry was prose —
"Registry schema migration test." Only 2 of 17 tasks declared anything
executable, and `archon requirements trace` reported **0 of 93 requirements
satisfied** despite the tasks and the requirements covering each other
exactly. The work was done; nothing could prove it.

```markdown
## Focused Tests

- `cargo test -p archon-trading registry_migration`
- `cargo test -p archon-trading coverage_matrix_gaps`
- `cargo test -p archon-cli --lib cli_args::tests::trading_data_coverage_parses`
```

`cargo test -p archon-trading registry_migration` proves something.
"Registry schema migration test." cannot.

The discrimination is mechanical, not a heuristic:

- The bullet must contain a **backticked** span.
- The **first whitespace-separated token inside the backticks** must be one of
  eighteen literals: `archon`, `bash`, `cargo`, `deno`, `go`, `gradle`,
  `just`, `make`, `mvn`, `node`, `npm`, `pnpm`, `pytest`, `python`,
  `python3`, `sh`, `tox`, `yarn`.
- Anything else is classified as prose. A backticked `/trading data coverage`
  is prose. A backticked `data list --json` is prose. An unbackticked
  `cargo test -p x` is prose.

A bullet may carry a sentence around the command; only the backticked span is
read. Prefer one command per bullet.

If a check genuinely has no runnable command, write the prose bullet anyway
and say what is unproven — but then do not expect the requirement it covers to
promote, and record the shortfall in `## Adversarial Review Notes`.

## 8. `## Files Expected to Change` — real paths, not prose

```markdown
## Files Expected to Change

- `crates/archon-trading/src/data_lake.rs`
- `crates/archon-trading/src/data_store.rs`
- `src/command/trading_data.rs`
- `src/cli_args/trading_market_actions.rs`
- `src/cli_args/tests.rs`
```

Backtick-quoted paths are what gets lifted. Two consumers read this section
and one of them requires the backticks absolutely: the traceability reader
takes only backticked spans, and a section with none yields no anchors at all,
so the requirements the task claims get no evidence and cannot promote.

A path qualifies when it contains `/` or carries a one-to-four-character
extension, and contains no whitespace.

Two anti-patterns, both from the reference corpus, both defects:

- **"Likely anchors: `a.rs`, `b.rs`, and the real TUI registry (discover it —
  do not assume)."** The hedge forces a repo-wide search at execution time,
  which is the shape of the padded-evidence defect: the agent reads
  everything, and reading everything is indistinguishable from reading
  nothing.
- **The same anchor list repeated across tasks.** Identical lists make the
  section useless as a dataflow signal — the topology lint treats these paths
  as *production*, so every task appears to write every file and the
  write-conflict lint reports noise instead of conflicts.

Name the files this task actually changes. If you genuinely do not know one,
say so in `## Adversarial Review Notes` and list the ones you do know here —
do not put the uncertainty in this section.

## 9. Task file structure

````markdown
# TASK-<DOMAIN>-<NNN> — <Title>

```yaml
<the block from §2>
```

## Purpose

One paragraph. What this task delivers and why the PRD needs it.

## Scope

### In

- Specific, bounded items.

### Out

- What this task explicitly does not do, so the next task can claim it.

## Files Expected to Change

- `backtick/quoted/real/path.rs`

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Secrets, credentials, local provider tokens.

## Acceptance Criteria

- Bulleted, checkable statements. Read verbatim by the engine and shown to
  the implementing and reviewing agents.

## Focused Tests

- `cargo test -p <crate> <filter>`

## Adversarial Review Notes

- What a reviewer should try to break, and what a plausible-looking wrong
  implementation would look like.

## Required Task Checklist

- implements (normative requirement IDs)
- scope
- files expected to change
- files forbidden to change
- acceptance criteria
- focused tests
- adversarial review notes
- explicit residual gaps with fail-closed behavior

## Global Constraints

- Keep changed and new files under 500 lines.
- No hardcoded secrets or provider credentials.
- No vague "later", "TBD", "probably", or "best effort" without a residual
  gap record naming what fails closed.
````

Section headings are matched case-insensitively at any heading level, and
bullets must start with `- ` or `* `. A heading not in this list is ignored,
silently — so a typo in `## Focused Tests` costs the whole section.

Do not hand-edit anything between `<!-- PRIOR-RUN-FINDINGS:BEGIN -->` and
`<!-- PRIOR-RUN-FINDINGS:END -->`. That block is appended by a previous run
and is deliberately excluded from the task's declarations; text you add there
does not count as anything you declared.

## 10. Decomposition method

1. **Read the PRD.** Extract every `REQ-<AREA>-<NNN>` and its section number.
2. **Group requirements into tasks** by deliverable, not by document order.
   One task is one clear deliverable, testable, roughly a day. Each task's
   `source_sections` is the set of PRD sections its requirements came from.
3. **Assign every requirement to at least one task.** Work through the list
   and check off each ID. An ID with no home is a task you have not written.
4. **Draw the graph.** `depends_on` where a task consumes another's artifact,
   `blocks` where it is more natural to say it forward. Check for cycles by
   hand before writing files.
5. **Write the contracts.** For each task, what artifact does it produce, and
   what command checks it? A task producing nothing checkable is a task whose
   requirements cannot promote — either give it a contract or move its
   requirements to a task that has one.
6. **Write the focused tests as commands.** Name the crate and the test
   filter. If the test does not exist yet, name what it will be called; the
   command is a contract with the implementer.
7. **Write the files-expected list as paths.**
8. **Set statuses.** `pending` for tasks with no dependencies, `blocked` for
   tasks with them.

## 11. Verify before handing off

Run these and report the output. They are cheap and each one catches a class
of error this document exists to prevent.

```
archon workflow lint --tasks tasks/PRD-<NAME>/
archon requirements trace --prd prds/PRD-<NAME>/PRD-<NAME>.md --tasks tasks/PRD-<NAME>/
```

The lint's `## requirement coverage` section will report
`no PRD found beside tasks/PRD-<NAME>/; skipped`, because it looks for the PRD
as a sibling of the task directory and the PRD lives under `prds/`. That is
expected and is not an error. The `requirements trace` invocation above takes
both paths explicitly and is the authoritative coverage and traceability
check — use its output.

Report, in the summary: the number of task files written, the number of
requirements claimed against the number the PRD defines, any unclaimed
requirement, any cited ID the PRD does not define, and every focused-test
entry that classified as prose.

## 12. Output requirements

1. Read the PRD with the Read tool before writing anything.
2. Create `tasks/PRD-<NAME>/` and write one `TASK-<DOMAIN>-<NNN>-<slug>.md`
   per task, flat, in that directory.
3. Keep each tool-call payload under 8,000 characters. Write one task file
   per call rather than batching. When the invoking skill supplies a fan-out
   execution model, that model governs WHO writes each file and in what order;
   this clause governs only the size and granularity of an individual write.
4. After writing every file, run the two commands in §11.
5. Print the list of paths created and the §11 summary. Do not print full task
   bodies into the conversation.
